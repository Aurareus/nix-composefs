//! NAR serialisation hash of a composefs tree, to check sealed content against
//! the `narHash` Nix recorded for the path.
//!
//! Hashing the *tree we are about to commit* (with external file contents read
//! back from the object store, through fs-verity) checks exactly what will be
//! served after the next boot, not merely what was on disk before import.

use std::{ffi::OsStr, fs::File, io::Read, os::unix::ffi::OsStrExt};

use anyhow::{Context, Result, bail};
use composefs::{
    fsverity::FsVerityHashValue,
    generic_tree::LeafContent,
    repository::Repository,
    tree::{Directory, FileSystem, Inode, RegularFile},
};
use openssl::sha::Sha256;

struct NarWriter {
    h: Sha256,
}

impl NarWriter {
    fn bytes(&mut self, b: &[u8]) {
        self.h.update(&(b.len() as u64).to_le_bytes());
        self.h.update(b);
        let pad = (8 - b.len() % 8) % 8;
        self.h.update(&[0u8; 8][..pad]);
    }
    fn s(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

fn leaf<ID: FsVerityHashValue>(
    w: &mut NarWriter,
    fs: &FileSystem<ID>,
    id: composefs::generic_tree::LeafId,
    repo: &Repository<ID>,
) -> Result<()> {
    let l = fs.leaf(id);
    w.s("(");
    match &l.content {
        LeafContent::Regular(f) => {
            w.s("type");
            w.s("regular");
            if l.stat.st_mode & 0o100 != 0 {
                w.s("executable");
                w.s("");
            }
            w.s("contents");
            match f {
                RegularFile::Inline(data) => w.bytes(data),
                RegularFile::External(oid, size) | RegularFile::ExternalNoVerity(oid, size) => {
                    w.h.update(&size.to_le_bytes());
                    let mut file = File::from(repo.open_object(oid)?);
                    let mut buf = vec![0u8; 1 << 20];
                    let mut total = 0u64;
                    loop {
                        let n = file.read(&mut buf).context("reading object for NAR hash")?;
                        if n == 0 {
                            break;
                        }
                        total += n as u64;
                        w.h.update(&buf[..n]);
                    }
                    if total != *size {
                        bail!("object {} has size {total}, expected {size}", oid.to_hex());
                    }
                    let pad = ((8 - size % 8) % 8) as usize;
                    w.h.update(&[0u8; 8][..pad]);
                }
                RegularFile::Sparse(_) => bail!("sparse file in store path"),
            }
        }
        LeafContent::Symlink(target) => {
            w.s("type");
            w.s("symlink");
            w.s("target");
            w.bytes(target.as_bytes());
        }
        _ => bail!("store paths may only contain regular files, symlinks and directories"),
    }
    w.s(")");
    Ok(())
}

fn dir<ID: FsVerityHashValue>(
    w: &mut NarWriter,
    fs: &FileSystem<ID>,
    d: &Directory<ID>,
    repo: &Repository<ID>,
) -> Result<()> {
    w.s("(");
    w.s("type");
    w.s("directory");
    // Directory entries are kept in a BTreeMap over raw bytes, which is the
    // same order NAR uses.
    for (name, inode) in d.sorted_entries() {
        w.s("entry");
        w.s("(");
        w.s("name");
        w.bytes(name.as_bytes());
        w.s("node");
        node(w, fs, inode, repo)?;
        w.s(")");
    }
    w.s(")");
    Ok(())
}

fn node<ID: FsVerityHashValue>(
    w: &mut NarWriter,
    fs: &FileSystem<ID>,
    inode: &Inode<ID>,
    repo: &Repository<ID>,
) -> Result<()> {
    match inode {
        Inode::Directory(d) => dir(w, fs, d, repo),
        Inode::Leaf(id, _) => leaf(w, fs, *id, repo),
    }
}

/// sha256 of the NAR serialisation of `name` (a top-level entry of `fs`).
pub fn nar_hash<ID: FsVerityHashValue>(
    fs: &FileSystem<ID>,
    name: &OsStr,
    repo: &Repository<ID>,
) -> Result<[u8; 32]> {
    let inode = fs
        .root
        .lookup(name)
        .with_context(|| format!("{} missing from tree", name.to_string_lossy()))?;
    let mut w = NarWriter { h: Sha256::new() };
    w.s("nix-archive-1");
    node(&mut w, fs, inode, repo)?;
    Ok(w.h.finish())
}

/// Parse Nix's `ValidPaths.hash` column ("sha256:<base16>" or "sha256:<nix32>").
pub fn parse_db_hash(s: &str) -> Result<[u8; 32]> {
    let rest = s
        .strip_prefix("sha256:")
        .with_context(|| format!("unsupported NAR hash {s}"))?;
    let mut out = [0u8; 32];
    if rest.len() == 64 {
        hex::decode_to_slice(rest, &mut out)?;
        return Ok(out);
    }
    if rest.len() == 52 {
        // nix32: custom base32, least significant digit last.
        const ALPHA: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";
        for (n, c) in rest.bytes().rev().enumerate() {
            let digit = ALPHA.iter().position(|&a| a == c).context("bad nix32")? as u32;
            let b = n * 5;
            let i = b / 8;
            let j = b % 8;
            out[i] |= (digit << j) as u8;
            if i + 1 < 32 {
                out[i + 1] |= (digit >> (8 - j)) as u8;
            }
        }
        return Ok(out);
    }
    bail!("unrecognised NAR hash encoding {s}")
}
