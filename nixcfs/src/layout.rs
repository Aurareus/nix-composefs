//! On-disk layout, relative to a root directory ("/" on a running system,
//! "/sysroot" in the initrd, or an image-builder mount point).

use std::path::{Path, PathBuf};

pub const OVERLAYFS_SUPER_MAGIC: i64 = 0x794c7630;
/// Name of the image ref that always points at the most recent seal.
pub const LATEST_REF: &str = "nixos/latest";
/// Kernel command line key carrying the trusted image digest.
pub const CMDLINE_KEY: &str = "nixcfs.image";
/// Where the initrd records what it mounted (visible as /run/nixcfs after switch-root).
pub const RUN_DIR: &str = "/run/nixcfs";

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self { root: root.as_ref().to_owned() }
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    /// composefs-rs repository (objects/, images/, meta.json).
    pub fn repo(&self) -> PathBuf {
        self.p("nix/.cfs")
    }
    /// Writable state: overlay upper/work, trash, caches, lock.
    pub fn rw(&self) -> PathBuf {
        self.p("nix/.cfs-rw")
    }
    pub fn upper(&self) -> PathBuf {
        self.rw().join("upper")
    }
    pub fn work(&self) -> PathBuf {
        self.rw().join("work")
    }
    pub fn trash(&self) -> PathBuf {
        self.rw().join("trash")
    }
    pub fn seal_cache(&self) -> PathBuf {
        self.rw().join("seal-cache.json")
    }
    pub fn lock(&self) -> PathBuf {
        self.rw().join("lock")
    }
    pub fn store(&self) -> PathBuf {
        self.p("nix/store")
    }
    pub fn db(&self) -> PathBuf {
        self.p("nix/var/nix/db/db.sqlite")
    }
    pub fn gc_lock(&self) -> PathBuf {
        self.p("nix/var/nix/gc.lock")
    }
}
