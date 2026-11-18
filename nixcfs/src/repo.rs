//! Small helpers around the composefs-rs repository.

use std::{
    collections::HashSet,
    ffi::OsString,
    fs::File,
    io::Read,
    os::{fd::OwnedFd, unix::ffi::OsStrExt},
    path::Path,
};

use anyhow::{Context, Result, bail};
use composefs::{
    fsverity::FsVerityHashValue,
    mount::erofs_mount,
    repository::{IntegrityPolicy, Repository, VerityState},
    tree::FileSystem,
};
use rustix::fs::{CWD, Dir, FileType};

/// Open the repository in secure mode, whatever meta.json says.
pub fn open_secure<ID: FsVerityHashValue>(path: &Path) -> Result<Repository<ID>> {
    let mut repo = Repository::<ID>::open_path(CWD, path)
        .with_context(|| format!("opening composefs repository {}", path.display()))?;
    repo.set_integrity_policy(IntegrityPolicy::Explicit(VerityState::Enabled))?;
    Ok(repo)
}

/// Read an image whose fs-verity digest must equal `digest`.
pub fn read_image<ID: FsVerityHashValue>(repo: &Repository<ID>, digest: &ID) -> Result<Vec<u8>> {
    let (fd, verified) = repo.open_digest(digest)?;
    if !verified {
        bail!("image {} is not fs-verity protected", digest.to_hex());
    }
    let mut data = vec![];
    File::from(fd).read_to_end(&mut data)?;
    Ok(data)
}

pub fn load_image<ID: FsVerityHashValue>(repo: &Repository<ID>, digest: &ID) -> Result<FileSystem<ID>> {
    let data = read_image(repo, digest)?;
    composefs::erofs::reader::erofs_to_filesystem::<ID>(&data)
        .with_context(|| format!("parsing image {}", digest.to_hex()))
}

/// Names in the root directory of an image, read through a (verified,
/// detached) EROFS mount.  Overlay whiteout stubs (V1 format) are skipped.
pub fn image_top_names<ID: FsVerityHashValue>(
    repo: &Repository<ID>,
    digest: &ID,
) -> Result<(HashSet<OsString>, rustix::fs::Stat)> {
    let (fd, verified) = repo.open_digest(digest)?;
    if !verified {
        bail!("image {} is not fs-verity protected", digest.to_hex());
    }
    let mnt: OwnedFd = erofs_mount(fd)?;
    // fsmount() returns an O_PATH-like fd; open the root for reading.
    let mnt = rustix::fs::openat(&mnt, ".", rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC, rustix::fs::Mode::empty())?;
    let root_stat = rustix::fs::fstat(&mnt)?;
    let mut names = HashSet::new();
    for e in Dir::read_from(&mnt)? {
        let e = e?;
        let n = e.file_name().to_bytes();
        if n == b"." || n == b".." {
            continue;
        }
        if e.file_type() == FileType::CharacterDevice && n.len() == 2 {
            continue; // whiteout stub "00".."ff"
        }
        names.insert(OsString::from(std::ffi::OsStr::from_bytes(n)));
    }
    Ok((names, root_stat))
}

/// Top-level names of an image, parsed in userspace (no erofs module needed).
pub fn image_top_names_userspace<ID: FsVerityHashValue>(repo: &Repository<ID>, digest: &ID) -> Result<HashSet<OsString>> {
    let fs = load_image(repo, digest)?;
    Ok(fs
        .root
        .entries()
        .filter(|(n, i)| !(n.len() == 2 && matches!(i, composefs::tree::Inode::Leaf(..)) && is_stub(&fs, i)))
        .map(|(n, _)| n.to_owned())
        .collect())
}

fn is_stub<ID: FsVerityHashValue>(fs: &FileSystem<ID>, i: &composefs::tree::Inode<ID>) -> bool {
    match i {
        composefs::tree::Inode::Leaf(id, _) => matches!(fs.leaf(*id).content, composefs::generic_tree::LeafContent::CharacterDevice(0)),
        _ => false,
    }
}

pub fn parse_digest<ID: FsVerityHashValue>(s: &str) -> Result<ID> {
    let s = s.strip_prefix("sha256:").or_else(|| s.strip_prefix("sha512:")).unwrap_or(s);
    ID::from_hex(s).map_err(|e| anyhow::anyhow!("bad digest {s}: {e:?}"))
}

/// Digest an image ref points at (images/refs/<name> -> ../../<hex>).
pub fn read_ref<ID: FsVerityHashValue>(repo_path: &Path, name: &str) -> Result<Option<ID>> {
    let p = repo_path.join("images/refs").join(name);
    match std::fs::read_link(&p) {
        Ok(t) => {
            let hex = t
                .file_name()
                .context("bad ref target")?
                .to_str()
                .context("bad ref target")?
                .to_owned();
            Ok(Some(parse_digest(&hex)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading ref {}", p.display())),
    }
}

pub fn set_ref<ID: FsVerityHashValue>(repo: &Repository<ID>, name: &str, digest: &ID) -> Result<()> {
    repo.symlink(format!("images/refs/{name}"), format!("images/{}", digest.to_hex()))?;
    Ok(())
}

/// All refs under images/refs/<prefix>/ as (name, digest-hex).
pub fn list_refs(repo_path: &Path, prefix: &str) -> Result<Vec<(String, String)>> {
    let dir = repo_path.join("images/refs").join(prefix);
    let mut out = vec![];
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for e in rd {
        let e = e?;
        let t = std::fs::read_link(e.path())?;
        out.push((
            format!("{prefix}/{}", e.file_name().to_string_lossy()),
            t.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        ));
    }
    Ok(out)
}

pub fn remove_ref(repo_path: &Path, name: &str) -> Result<()> {
    let p = repo_path.join("images/refs").join(name);
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
