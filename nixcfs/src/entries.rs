//! Boot Loader Specification entries: stamp the image digest on every NixOS
//! entry, drop entries whose system is not in the image, and collect garbage.

use std::{
    collections::{BTreeSet, HashSet},
    ffi::OsString,
    io::Write,
    path::Path,
};

use anyhow::{Context, Result};
use composefs::fsverity::FsVerityHashValue;
use log::{info, warn};
use serde::Serialize;

use crate::{layout::*, repo::*};

#[derive(Debug, Default, Serialize)]
pub struct EntriesReport {
    pub updated: Vec<String>,
    pub removed_unsealed: Vec<String>,
    pub removed_without_nixcfs: Vec<String>,
}

fn entry_files(esp: &Path) -> Result<Vec<std::path::PathBuf>> {
    let dir = esp.join("loader/entries");
    let mut v = vec![];
    for e in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let p = e?.path();
        let n = p.file_name().unwrap().to_string_lossy();
        if n.starts_with("nixos-") && n.ends_with(".conf") {
            v.push(p);
        }
    }
    v.sort();
    Ok(v)
}

/// The store path name of the system an entry boots (from init=).
fn entry_system(text: &str) -> Option<String> {
    let opts = text.lines().find_map(|l| l.strip_prefix("options "))?;
    let init = opts.split_ascii_whitespace().find_map(|w| w.strip_prefix("init="))?;
    let rest = init.strip_prefix("/nix/store/")?;
    Some(rest.split('/').next()?.to_owned())
}

pub fn entry_digests(esp: &Path) -> Result<BTreeSet<String>> {
    let mut s = BTreeSet::new();
    for p in entry_files(esp)? {
        let t = std::fs::read_to_string(&p)?;
        if let Some(opts) = t.lines().find_map(|l| l.strip_prefix("options ")) {
            if let Some(d) = crate::boot::cmdline_digest(opts) {
                s.insert(d);
            }
        }
    }
    Ok(s)
}

/// Stamp `image` on every NixOS entry.  With `prune`, also remove entries that
/// cannot boot with it (system not in the image, or built without nixcfs).
pub fn update_entries<ID: FsVerityHashValue>(layout: &Layout, esp: &Path, image: &ID, prune: bool) -> Result<EntriesReport> {
    let repo = open_secure::<ID>(&layout.repo())?;
    let names: HashSet<OsString> = image_top_names_userspace(&repo, image)?;
    let mut rep = EntriesReport::default();
    for p in entry_files(esp)? {
        let text = std::fs::read_to_string(&p)?;
        let fname = p.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(sys) = entry_system(&text) {
            if prune && !names.contains(&OsString::from(&sys)) {
                warn!("{fname}: system {sys} is not in the store image; removing entry");
                std::fs::remove_file(&p)?;
                rep.removed_unsealed.push(fname);
                continue;
            }
            // A system built without the nixcfs module cannot mount the
            // store (its initrd knows nothing about it): such an entry would
            // not boot once the plain store is gone.
            let marker = layout.root.join("nix/store").join(&sys).join("nixcfs");
            let sysdir = layout.root.join("nix/store").join(&sys);
            if prune && sysdir.exists() && !marker.exists() {
                warn!("{fname}: system {sys} predates nixcfs (no composefs support in its initrd); removing entry");
                std::fs::remove_file(&p)?;
                rep.removed_without_nixcfs.push(fname);
                continue;
            }
        }
        let mut out = String::new();
        for line in text.lines() {
            if let Some(opts) = line.strip_prefix("options ") {
                let kept: Vec<&str> = opts
                    .split_ascii_whitespace()
                    .filter(|w| !w.starts_with(&format!("{CMDLINE_KEY}=")))
                    .collect();
                out.push_str(&format!("options {} {CMDLINE_KEY}={}\n", kept.join(" "), image.to_hex()));
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        if out != text {
            let tmp = p.with_extension("conf.tmp");
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(out.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, &p)?;
            rep.updated.push(fname);
        }
    }
    // Make the renames durable on the (FAT) ESP.
    std::fs::File::open(esp.join("loader/entries"))?.sync_all().ok();
    rustix::fs::sync();
    Ok(rep)
}

#[derive(Debug, Serialize)]
pub struct GcReport {
    pub kept_images: Vec<String>,
    pub dropped_refs: Vec<String>,
    pub objects_removed: u64,
    pub bytes_freed: u64,
    pub images_removed: u64,
}

/// Keep: the latest image, every image a boot entry names, and the image the
/// running system booted from (its objects back the running store; they may
/// only go once nothing has it mounted, i.e. after the next boot).
///
/// composefs-rs also tries to keep mounted images alive via a flock on the
/// image file, but that lock does not survive the initrd (or kernels that
/// reject an fd as the erofs source), so we do not rely on it.
pub fn gc<ID: FsVerityHashValue>(layout: &Layout, esp: Option<&Path>) -> Result<GcReport> {
    let repo_path = layout.repo();
    let repo = open_secure::<ID>(&repo_path)?;
    let mut keep: BTreeSet<String> = BTreeSet::new();
    if let Some(l) = read_ref::<ID>(&repo_path, LATEST_REF)? {
        keep.insert(l.to_hex());
    }
    if let Some(esp) = esp {
        keep.extend(entry_digests(esp)?);
    }
    if layout.root == Path::new("/") {
        if let Ok(b) = std::fs::read_to_string(Path::new(RUN_DIR).join("image")) {
            keep.insert(b.trim().to_owned());
        }
    }
    let mut dropped = vec![];
    for (name, hex) in list_refs(&repo_path, "nixos/boot")? {
        if !keep.contains(&hex) {
            remove_ref(&repo_path, &name)?;
            dropped.push(name);
        }
    }
    for hex in &keep {
        let d: ID = parse_digest(hex)?;
        if repo_path.join("images").join(hex).exists() {
            set_ref(&repo, &format!("nixos/boot/{hex}"), &d)?;
        } else {
            warn!("boot entry names image {hex}, which is not in the repository");
        }
    }
    let r = repo.gc(&[])?;
    info!("gc: {r:?}");
    Ok(GcReport {
        kept_images: keep.into_iter().collect(),
        dropped_refs: dropped,
        objects_removed: r.objects_removed,
        bytes_freed: r.objects_bytes,
        images_removed: r.images_pruned,
    })
}
