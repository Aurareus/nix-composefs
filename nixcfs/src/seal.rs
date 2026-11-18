//! Sealing: build a new composefs image containing every valid store path.
//!
//! Paths already present in the previous image are reused without touching
//! their data; new paths are scanned from where Nix physically wrote them
//! (the overlay upper dir, or a plain /nix/store before the first boot),
//! their file data is imported into the object store without copying
//! (hardlink + in-place fs-verity on ext4, reflink on btrfs), and their NAR
//! hash is checked against the Nix database.

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs::File,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context, Result, bail};
use composefs::{
    fs::{HardlinkBehavior, ObjectStore, ReadFilesystemOpts, read_filesystem_with_opts},
    fsverity::FsVerityHashValue,
    generic_tree::{LeafContent, Stat},
    repository::{ImportContext, Repository},
    tree::{Directory, FileSystem, Inode, RegularFile},
};
use log::{info, warn};
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, openat, statat};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::{layout::*, nar, repo::*};

/// Imports file data by reflink or hardlink (never copies unless both fail).
struct ZeroCopyStore<ID: FsVerityHashValue> {
    repo: Arc<Repository<ID>>,
    sem: Arc<Semaphore>,
    stats: Arc<Mutex<ImportStats>>,
}

#[derive(Default, Debug, Serialize)]
pub struct ImportStats {
    pub reflinked: u64,
    pub hardlinked: u64,
    pub copied: u64,
    pub already_present: u64,
    pub bytes: u64,
}

impl<ID: FsVerityHashValue> ObjectStore<ID> for ZeroCopyStore<ID> {
    fn ensure_object_from_fd(&self, fd: OwnedFd, size: u64) -> Result<ID> {
        let f = File::from(fd);
        let mut ctx = ImportContext::default();
        let (id, how) = self.repo.ensure_object_from_file(&f, size, &mut ctx)?;
        let mut s = self.stats.lock().unwrap();
        use composefs::repository::ObjectStoreMethod::*;
        match how {
            Reflinked => s.reflinked += 1,
            Hardlinked => s.hardlinked += 1,
            Copied => s.copied += 1,
            AlreadyPresent => s.already_present += 1,
        }
        s.bytes += size;
        Ok(id)
    }
    fn write_semaphore(&self) -> Arc<Semaphore> {
        self.sem.clone()
    }
}

#[derive(Debug, Clone)]
pub struct SealOptions {
    pub layout: Layout,
    /// Physical directories to look for unsealed store paths in, in order.
    /// Default: the overlay upper dir if /nix/store is our overlay, else
    /// /nix/store itself.
    pub sources: Vec<PathBuf>,
    /// Image the running system booted from (for telling copy-ups apart from
    /// re-added paths); default: read from /run/nixcfs/image.
    pub booted: Option<String>,
    pub nar_check: bool,
    pub jobs: usize,
    /// Only seal these paths (plus nothing else); for tests/benchmarks.
    pub only: Option<HashSet<String>>,
}

#[derive(Debug, Serialize)]
pub struct SealReport {
    pub image: String,
    pub previous: Option<String>,
    pub valid_paths: usize,
    pub reused_paths: usize,
    pub scanned_paths: usize,
    pub missing_paths: Vec<String>,
    pub reverted_paths: Vec<String>,
    pub import: ImportStats,
    pub seconds: f64,
}

/// Cache: store path name -> identity of the upper copy that was sealed, so
/// a later seal can reuse the image subtree instead of rescanning it.
#[derive(Default, Serialize, Deserialize)]
struct SealCache {
    entries: HashMap<String, (u64, u64, i64, i64)>, // dev, ino, ctime, ctime_nsec
}

fn ident(m: &std::fs::Metadata) -> (u64, u64, i64, i64) {
    (m.dev(), m.ino(), m.ctime(), m.ctime_nsec())
}

/// Is an upper-layer entry a complete, standalone copy of the path (as
/// opposed to the residue of a copy-up of a path in the lower layer)?
fn upper_is_complete(path: &Path, md: &std::fs::Metadata) -> Result<bool> {
    let ft = md.file_type();
    if ft.is_dir() {
        // A directory created over a whiteout is marked opaque.
        let opaque = getxattr(path, "trusted.overlay.opaque")?;
        Ok(opaque.as_deref() == Some(b"y"))
    } else if ft.is_file() {
        // A metadata-only copy-up of a lower file carries a metacopy xattr.
        Ok(getxattr(path, "trusted.overlay.metacopy")?.is_none())
    } else {
        Ok(true)
    }
}

pub fn getxattr(path: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let mut buf = vec![0u8; 256];
    match rustix::fs::lgetxattr(path, name, &mut buf) {
        Ok(n) => {
            buf.truncate(n);
            Ok(Some(buf))
        }
        Err(rustix::io::Errno::NODATA) | Err(rustix::io::Errno::OPNOTSUPP) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("getxattr {name} on {}", path.display())),
    }
}

pub fn is_whiteout(md: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    md.file_type().is_char_device() && md.rdev() == 0
}

fn read_valid_paths(db: &Path, store_dir: &str) -> Result<HashMap<String, String>> {
    let uri = format!("file:{}?mode=ro", db.display());
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("opening Nix database {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(60))?;
    let mut stmt = conn.prepare("SELECT path, hash FROM ValidPaths")?;
    let mut out = HashMap::new();
    let prefix = format!("{store_dir}/");
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (path, hash) = row?;
        let name = path
            .strip_prefix(&prefix)
            .with_context(|| format!("valid path {path} not in {store_dir}"))?;
        out.insert(name.to_owned(), hash);
    }
    Ok(out)
}

/// Move the subtree of `src` named `name` into `dst` (leaves are appended).
fn graft<ID: FsVerityHashValue>(dst: &mut FileSystem<ID>, src: &FileSystem<ID>, inode: &Inode<ID>) -> Inode<ID> {
    match inode {
        Inode::Directory(d) => {
            let mut nd = Directory::new(d.stat.clone());
            for (n, child) in d.entries() {
                let c = graft(dst, src, child);
                nd.insert(n, c);
            }
            Inode::Directory(Box::new(nd))
        }
        Inode::Leaf(id, _) => {
            let l = src.leaf(*id);
            let nid = dst.push_leaf(l.stat.clone(), l.content.clone());
            Inode::leaf(nid)
        }
    }
}

fn clear_xattrs<ID: FsVerityHashValue>(fs: &mut FileSystem<ID>) {
    // Nix store contents carry no extended attributes; overlay bookkeeping
    // (trusted.overlay.*) must not leak into the image.
    fs.filter_xattrs(|_| false);
}

/// Scan one store path from a physical directory into its own FileSystem
/// whose root entry `name` is the path.
async fn scan_path<ID: FsVerityHashValue>(
    srcdir: PathBuf,
    name: OsString,
    store: Arc<ZeroCopyStore<ID>>,
) -> Result<FileSystem<ID>> {
    let full = srcdir.join(&name);
    let md = std::fs::symlink_metadata(&full)?;
    let mut out = FileSystem::<ID>::new(Stat::uninitialized());
    let inode = if md.is_dir() {
        let dirfd = openat(CWD, &srcdir, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
        let sub = read_filesystem_with_opts::<ID>(
            dirfd,
            PathBuf::from(&name),
            ReadFilesystemOpts {
                store: Some(store.clone() as Arc<dyn ObjectStore<ID>>),
                semaphore: Some(store.sem.clone()),
                hardlinks: HardlinkBehavior::Break,
            },
        )
        .await
        .with_context(|| format!("scanning {}", full.display()))?;
        let root = Inode::Directory(Box::new(sub.root.clone()));
        graft(&mut out, &sub, &root)
    } else {
        let stat = Stat {
            st_mode: md.mode() & 0o7777,
            st_uid: md.uid(),
            st_gid: md.gid(),
            st_mtim_sec: md.mtime(),
            st_mtim_nsec: md.mtime_nsec() as u32,
            xattrs: Default::default(),
        };
        let content = if md.file_type().is_symlink() {
            LeafContent::Symlink(std::fs::read_link(&full)?.into_os_string().into_boxed_os_str())
        } else if md.is_file() {
            let size = md.len();
            if size as usize <= composefs::INLINE_CONTENT_MAX_V0 {
                LeafContent::Regular(RegularFile::Inline(std::fs::read(&full)?.into_boxed_slice()))
            } else {
                let fd: OwnedFd = File::open(&full)?.into();
                let st = store.clone();
                let id = tokio::task::spawn_blocking(move || st.ensure_object_from_fd(fd, size)).await??;
                LeafContent::Regular(RegularFile::External(id, size))
            }
        } else {
            bail!("{}: unsupported file type at top level", full.display());
        };
        Inode::leaf(out.push_leaf(stat, content))
    };
    out.root.insert(&name, inode);
    clear_xattrs(&mut out);
    Ok(out)
}

fn read_booted(layout: &Layout, opt: &Option<String>) -> Option<String> {
    if let Some(b) = opt {
        return Some(b.clone());
    }
    if layout.root != Path::new("/") {
        return None;
    }
    std::fs::read_to_string(Path::new(RUN_DIR).join("image")).ok().map(|s| s.trim().to_owned())
}

pub fn store_is_overlay(store: &Path) -> bool {
    rustix::fs::statfs(store).map(|s| s.f_type as i64 == OVERLAYFS_SUPER_MAGIC).unwrap_or(false)
}

pub async fn seal<ID: FsVerityHashValue>(opts: SealOptions) -> Result<SealReport> {
    let t0 = Instant::now();
    let layout = &opts.layout;
    std::fs::create_dir_all(layout.rw())?;
    let lock = File::create(layout.lock())?;
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)?;

    // Keep the Nix garbage collector out while we work (it takes this lock
    // exclusively); builds only take it shared, so they keep running.
    let gclock = File::options().read(true).write(true).create(true).open(layout.gc_lock())?;
    info!("waiting for Nix GC lock (shared)");
    rustix::fs::flock(&gclock, rustix::fs::FlockOperation::LockShared)?;

    let repo = Arc::new(open_secure::<ID>(&layout.repo())?);
    let repo_path = layout.repo();

    let mut valid = read_valid_paths(&layout.db(), "/nix/store")?;
    if let Some(only) = &opts.only {
        valid.retain(|k, _| only.contains(k));
    }
    info!("{} valid store paths", valid.len());

    let previous: Option<ID> = read_ref(&repo_path, LATEST_REF)?;
    let mut fs: FileSystem<ID> = match &previous {
        Some(d) => load_image(&repo, d)?,
        None => FileSystem::new(Stat::uninitialized()),
    };
    let base_names: HashSet<OsString> = fs.root.entries().map(|(n, _)| n.to_owned()).collect();

    let booted_names: HashSet<OsString> = match read_booted(layout, &opts.booted) {
        Some(b) => image_top_names(&repo, &parse_digest::<ID>(&b)?)?.0,
        None => HashSet::new(),
    };
    let mut booted_fs: Option<FileSystem<ID>> = None;

    let sources: Vec<PathBuf> = if !opts.sources.is_empty() {
        opts.sources.clone()
    } else if store_is_overlay(&layout.store()) {
        vec![layout.upper()]
    } else {
        vec![layout.store()]
    };
    info!("sources: {sources:?}");

    let mut cache: SealCache = std::fs::read(layout.seal_cache())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    // Decide, for each valid path, where its content comes from.
    let mut to_scan: Vec<(PathBuf, OsString)> = vec![];
    let mut keep_from_base: HashSet<OsString> = HashSet::new();
    let mut from_booted: Vec<OsString> = vec![];
    let mut missing = vec![];
    let mut reverted = vec![];
    let mut names: Vec<&String> = valid.keys().collect();
    names.sort();
    for name in names {
        let os = OsString::from(name);
        let mut found = None;
        for s in &sources {
            let p = s.join(name);
            if let Ok(md) = std::fs::symlink_metadata(&p) {
                found = Some((s.clone(), p, md));
                break;
            }
        }
        match found {
            Some((_, p, md)) if is_whiteout(&md) => {
                // Hidden in the running store although Nix thinks it is valid.
                if base_names.contains(&os) {
                    warn!("{name}: whiteout in upper layer but valid; restoring sealed copy");
                    keep_from_base.insert(os);
                } else {
                    warn!("{name}: valid but deleted ({})", p.display());
                    missing.push(name.clone());
                }
            }
            Some((src, p, md)) => {
                let complete = !booted_names.contains(&os) || upper_is_complete(&p, &md)?;
                if !complete {
                    // Residue of modifying a sealed path at runtime: the sealed
                    // content wins.
                    warn!("{name}: modified at runtime (overlay copy-up); keeping sealed content");
                    reverted.push(name.clone());
                    if base_names.contains(&os) {
                        keep_from_base.insert(os);
                    } else {
                        from_booted.push(os);
                    }
                } else if base_names.contains(&os) && cache.entries.get(name) == Some(&ident(&md)) {
                    keep_from_base.insert(os);
                } else {
                    to_scan.push((src, os));
                }
            }
            None if base_names.contains(&os) => {
                keep_from_base.insert(os);
            }
            None => {
                warn!("{name}: valid in Nix DB but not found in {sources:?}; not sealed");
                missing.push(name.clone());
            }
        }
    }

    // Drop dead and to-be-replaced paths from the base tree.
    fs.root.retain_top_level(|n| keep_from_base.contains(OsStr::new(n)));
    let reused = keep_from_base.len();

    if !from_booted.is_empty() {
        let b = read_booted(layout, &opts.booted).context("no booted image")?;
        booted_fs = Some(load_image(&repo, &parse_digest::<ID>(&b)?)?);
    }
    if let Some(bfs) = &booted_fs {
        for n in &from_booted {
            match bfs.root.lookup(n) {
                Some(inode) => {
                    let i = graft(&mut fs, bfs, inode);
                    fs.root.insert(n, i);
                }
                None => missing.push(n.to_string_lossy().into_owned()),
            }
        }
    }

    // Scan new paths, a few at a time (each scan is itself parallel).
    let stats = Arc::new(Mutex::new(ImportStats::default()));
    let store = Arc::new(ZeroCopyStore {
        repo: repo.clone(),
        sem: Arc::new(Semaphore::new(opts.jobs)),
        stats: stats.clone(),
    });
    info!("scanning {} new store paths", to_scan.len());
    let mut set = tokio::task::JoinSet::new();
    let mut iter = to_scan.clone().into_iter();
    let mut scanned = 0usize;
    let mut new_cache = SealCache::default();
    let path_sem = Arc::new(Semaphore::new(opts.jobs.max(1)));
    loop {
        while set.len() < opts.jobs.max(1) * 2 {
            let Some((src, name)) = iter.next() else { break };
            let st = store.clone();
            let permit = path_sem.clone().acquire_owned().await?;
            set.spawn(async move {
                let _p = permit;
                let r = scan_path::<ID>(src.clone(), name.clone(), st).await;
                (src, name, r)
            });
        }
        let Some(res) = set.join_next().await else { break };
        let (src, name, r) = res?;
        let sub = r?;
        let inode = sub.root.lookup(&name).context("scan lost root")?;
        let i = graft(&mut fs, &sub, inode);
        fs.root.insert(&name, i);
        if let Ok(md) = std::fs::symlink_metadata(src.join(&name)) {
            new_cache.entries.insert(name.to_string_lossy().into_owned(), ident(&md));
        }
        scanned += 1;
        if scanned % 500 == 0 {
            info!("scanned {scanned}/{}", to_scan.len());
        }
    }

    // Check scanned paths against the NAR hashes Nix recorded.
    if opts.nar_check {
        let t = Instant::now();
        let fsr = &fs;
        let repo2 = repo.clone();
        let bad: Vec<String> = std::thread::scope(|sc| {
            let chunks: Vec<Vec<&(PathBuf, OsString)>> = {
                let n = opts.jobs.max(1);
                let mut c = vec![vec![]; n];
                for (i, x) in to_scan.iter().enumerate() {
                    c[i % n].push(x);
                }
                c
            };
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    let repo2 = repo2.clone();
                    let valid = &valid;
                    sc.spawn(move || {
                        let mut bad = vec![];
                        for (_, name) in chunk {
                            let want = nar::parse_db_hash(&valid[&*name.to_string_lossy()]);
                            let got = nar::nar_hash(fsr, name, &repo2);
                            match (want, got) {
                                (Ok(w), Ok(g)) if w == g => {}
                                (w, g) => bad.push(format!("{}: want {:?} got {:?}", name.to_string_lossy(), w.map(hex::encode), g.map(hex::encode))),
                            }
                        }
                        bad
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        info!("NAR hash check of {} paths took {:.1}s", to_scan.len(), t.elapsed().as_secs_f64());
        if !bad.is_empty() {
            for b in &bad {
                log::error!("NAR hash mismatch: {b}");
            }
            bail!("{} store paths do not match their NAR hash in the Nix database; refusing to seal", bad.len());
        }
    }

    // Root directory metadata: what Nix/NixOS expect for /nix/store.
    let gid = std::fs::metadata(layout.store()).map(|m| m.gid()).unwrap_or(30000);
    fs.set_root_stat(Stat {
        st_mode: 0o1775,
        st_uid: 0,
        st_gid: gid,
        st_mtim_sec: 0,
        st_mtim_nsec: 0,
        xattrs: Default::default(),
    });
    fs.compact();

    // Every external object the image references must exist.
    let objects = layout.repo().join("objects");
    let objfd = openat(CWD, &objects, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    let mut nobj = 0u64;
    for l in &fs.leaves {
        if let LeafContent::Regular(RegularFile::External(id, size)) = &l.content {
            let st = statat(&objfd, id.to_object_pathname(), AtFlags::empty())
                .with_context(|| format!("object {} missing", id.to_hex()))?;
            if st.st_size as u64 != *size || FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
                bail!("object {} has wrong size/type", id.to_hex());
            }
            nobj += 1;
        }
    }
    info!("{nobj} external file references checked");

    let image = fs.commit_image(&repo, Some(LATEST_REF))?;
    repo.sync()?;

    // Persist the scan cache (merged with still-valid old entries).
    for (k, v) in cache.entries.drain() {
        if keep_from_base.contains(OsStr::new(&k)) {
            new_cache.entries.entry(k).or_insert(v);
        }
    }
    let tmp = layout.seal_cache().with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(&new_cache)?)?;
    std::fs::rename(&tmp, layout.seal_cache())?;
    drop(gclock);

    let import = std::mem::take(&mut *stats.lock().unwrap());
    Ok(SealReport {
        image: image.to_hex(),
        previous: previous.map(|p| p.to_hex()),
        valid_paths: valid.len(),
        reused_paths: reused,
        scanned_paths: scanned,
        missing_paths: missing,
        reverted_paths: reverted,
        import,
        seconds: t0.elapsed().as_secs_f64(),
    })
}

#[allow(dead_code)]
fn _assert_send<T: Send>() {}
#[allow(dead_code)]
fn _fd(_: impl AsFd) {}
