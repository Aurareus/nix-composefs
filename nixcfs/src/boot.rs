//! Boot-time mount of /nix/store (runs in the initrd against /sysroot).
//!
//! Security-relevant steps, in order:
//!  1. the image digest comes from the kernel command line (trust anchor);
//!  2. the image file's measured fs-verity digest must equal it;
//!  3. the writable upper layer is scrubbed so that it cannot shadow any
//!     name present in the image (only whiteouts may remain for those);
//!  4. overlayfs is mounted with verity=require, so every data file of the
//!     image is checked against the digest recorded in the image.

use std::{
    collections::HashSet,
    ffi::OsString,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result, bail};
use composefs::{
    fsverity::FsVerityHashValue,
    mount::{MountOptions, mount_at},
};
use log::{info, warn};
use rustix::fs::{CWD, Mode, OFlags, openat};
use serde::Serialize;

use crate::{
    layout::*,
    repo::*,
    seal::{getxattr, is_whiteout},
};

pub fn cmdline_digest(cmdline: &str) -> Option<String> {
    cmdline
        .split_ascii_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{CMDLINE_KEY}=")).map(str::to_owned))
}

#[derive(Debug, Default, Serialize)]
pub struct ScrubReport {
    pub removed_shadowing: Vec<String>,
    pub kept_whiteouts: usize,
    pub removed_stale_whiteouts: usize,
    pub kept_unsealed: usize,
    pub root_xattrs_removed: Vec<String>,
    pub legacy_moved_to_upper: usize,
    pub legacy_trashed: bool,
}

fn trash_dir(layout: &Layout) -> Result<PathBuf> {
    let t = layout.trash().join(format!(
        "{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&t)?;
    Ok(t)
}

/// Remove every trusted.overlay.* xattr from a directory.
fn strip_overlay_xattrs(p: &Path) -> Result<Vec<String>> {
    let mut buf = vec![0u8; 65536];
    let n = rustix::fs::llistxattr(p, &mut buf)?;
    let mut removed = vec![];
    for name in buf[..n].split(|&b| b == 0).filter(|s| !s.is_empty()) {
        let s = String::from_utf8_lossy(name).into_owned();
        if s.starts_with("trusted.overlay.") {
            rustix::fs::lremovexattr(p, name)?;
            removed.push(s);
        }
    }
    Ok(removed)
}

/// Make sure nothing in `upper` can shadow a name in `sealed`.
pub fn scrub_upper(layout: &Layout, sealed: &HashSet<OsString>, rep: &mut ScrubReport) -> Result<()> {
    let upper = layout.upper();
    let mut trash: Option<PathBuf> = None;
    rep.root_xattrs_removed = strip_overlay_xattrs(&upper)?;
    for e in std::fs::read_dir(&upper)? {
        let e = e?;
        let name = e.file_name();
        let md = std::fs::symlink_metadata(e.path())?;
        let wh = is_whiteout(&md);
        if sealed.contains(&name) {
            if wh {
                rep.kept_whiteouts += 1;
            } else {
                // Anything else here would be served instead of the verified
                // copy: a sealed duplicate, a runtime modification or tampering.
                let t = match &trash {
                    Some(t) => t.clone(),
                    None => {
                        let t = trash_dir(layout)?;
                        trash = Some(t.clone());
                        t
                    }
                };
                std::fs::rename(e.path(), t.join(&name))
                    .with_context(|| format!("moving {} to trash", e.path().display()))?;
                rep.removed_shadowing.push(name.to_string_lossy().into_owned());
            }
        } else if wh {
            std::fs::remove_file(e.path())?;
            rep.removed_stale_whiteouts += 1;
        } else {
            rep.kept_unsealed += 1;
        }
    }
    // Verify the invariant rather than trusting the loop above.
    for e in std::fs::read_dir(&upper)? {
        let e = e?;
        if sealed.contains(&e.file_name()) && !is_whiteout(&std::fs::symlink_metadata(e.path())?) {
            bail!("upper layer still shadows {}", e.file_name().to_string_lossy());
        }
    }
    if let Some(x) = getxattr(&upper, "trusted.overlay.opaque")? {
        bail!("upper root still opaque ({x:?})");
    }
    Ok(())
}

/// A plain (pre-composefs) /nix/store directory on disk: move it aside;
/// unsealed paths go to the upper layer, sealed ones to the trash.
fn migrate_legacy_store(layout: &Layout, sealed: &HashSet<OsString>, rep: &mut ScrubReport) -> Result<()> {
    let store = layout.store();
    let mut entries = std::fs::read_dir(&store)?.peekable();
    if entries.peek().is_none() {
        return Ok(());
    }
    info!("migrating plain {} into the composefs layout", store.display());
    let legacy = trash_dir(layout)?.join("legacy-store");
    let md = std::fs::metadata(&store)?;
    std::fs::rename(&store, &legacy)?;
    std::fs::create_dir(&store)?;
    std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o755))?;
    let _ = md;
    for e in std::fs::read_dir(&legacy)? {
        let e = e?;
        let name = e.file_name();
        if !sealed.contains(&name) && !layout.upper().join(&name).exists() {
            std::fs::rename(e.path(), layout.upper().join(&name))?;
            rep.legacy_moved_to_upper += 1;
        }
    }
    rep.legacy_trashed = true;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct BootReport {
    pub image: String,
    pub scrub: ScrubReport,
    pub seconds: f64,
}

pub fn boot<ID: FsVerityHashValue>(layout: &Layout, digest: &str, target: &Path) -> Result<BootReport> {
    let t0 = Instant::now();
    let digest_id: ID = parse_digest(digest)?;
    let repo = open_secure::<ID>(&layout.repo())?;

    // (1)+(2): verified image, read its top-level names.
    let (sealed, root_stat) = image_top_names(&repo, &digest_id).context("opening the store image named on the kernel command line")?;
    info!("image {} has {} store paths", digest_id.to_hex(), sealed.len());

    for d in [layout.rw(), layout.upper(), layout.work(), layout.trash()] {
        std::fs::create_dir_all(&d)?;
    }
    std::fs::set_permissions(layout.rw(), std::fs::Permissions::from_mode(0o700))?;
    let mut rep = ScrubReport::default();
    if !store_is_mountpoint(&layout.store())? {
        migrate_legacy_store(layout, &sealed, &mut rep)?;
    } else {
        bail!("{} is already a mount point", layout.store().display());
    }

    // (3)
    scrub_upper(layout, &sealed, &mut rep)?;
    let upper = layout.upper();
    rustix::fs::chown(&upper, Some(rustix::fs::Uid::from_raw(root_stat.st_uid)), Some(rustix::fs::Gid::from_raw(root_stat.st_gid)))?;
    std::fs::set_permissions(&upper, std::fs::Permissions::from_mode(root_stat.st_mode & 0o7777))?;
    if !rep.removed_shadowing.is_empty() {
        warn!("removed {} upper-layer entries shadowing sealed paths", rep.removed_shadowing.len());
    }

    // (4)
    let upfd = openat(CWD, &upper, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    let workfd = openat(CWD, layout.work(), OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    let mut mo = MountOptions::default();
    mo.set_overlay(upfd, workfd).set_read_write(true);
    let mnt = repo.mount_digest(&digest_id, &mo)?;
    mount_at(&mnt, CWD, target).with_context(|| format!("attaching store at {}", target.display()))?;
    info!("mounted {} on {}", digest_id.to_hex(), target.display());
    Ok(BootReport { image: digest_id.to_hex(), scrub: rep, seconds: t0.elapsed().as_secs_f64() })
}

pub fn store_is_mountpoint(p: &Path) -> Result<bool> {
    let me = std::fs::symlink_metadata(p)?;
    let parent = std::fs::symlink_metadata(p.join(".."))?;
    if me.dev() != parent.dev() {
        return Ok(true);
    }
    // Same device can still be a bind/overlay; check mountinfo.
    let mi = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let canon = std::fs::canonicalize(p)?;
    Ok(mi.lines().any(|l| l.split(' ').nth(4) == Some(canon.to_str().unwrap_or(""))))
}
