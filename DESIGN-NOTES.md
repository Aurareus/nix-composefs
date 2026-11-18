# Design notes (working document; RESULTS.md is the final write-up)

## Layout on disk (root fs must support fs-verity: ext4 -O verity, or btrfs)

```
/nix/.cfs/                 composefs-rs repository (sha256 fs-verity)
  objects/xx/yyyy…         file contents, named by fs-verity digest, verity enabled
  images/<digest>          EROFS metadata images (also objects)
  images/refs/…            refs keeping images alive for GC
/nix/.cfs-upper/upper      overlay upper layer: store paths added since last seal
/nix/.cfs-upper/work       overlay workdir
/nix/store                 mountpoint (empty directory on disk)
```

`/nix/store` = overlayfs(
  upperdir = /nix/.cfs-upper/upper,
  lowerdir+ = <EROFS image, detached mount>,
  datadir+  = /nix/.cfs/objects,
  metacopy=on, redirect_dir=on, verity=require)

## Seal (run by the bootloader-install step of nixos-rebuild, and after GC)
1. Take Nix's GC lock *shared* (GC cannot run concurrently; builds can).
2. Valid paths = `SELECT path FROM ValidPaths` (Nix DB, read-only).
3. New image = for each valid path: subtree from the previous sealed image if it
   is there and not replaced in upper; otherwise scan it from the upper dir
   (physical path on the root fs), importing file data with
   `ensure_object_from_file` (hardlink + in-place fs-verity on ext4, reflink on btrfs:
   no data copy).
4. Write image to repo, verify every referenced object exists.
5. Write `composefs digest` into every boot entry (systemd-boot BLS entries).
6. Drop refs to images no entry references; composefs-rs GC of objects.

Every boot entry uses the latest image. The latest image contains every valid
path at seal time, which includes the closures of all generations that have
entries (they are GC roots), so bootloader rollback and `switch --rollback` both
work with one image.

## Boot (systemd initrd)
1. Mount root at /sysroot as usual.
2. Read `nixcfs.image=<sha256 digest>` from the kernel command line (trust anchor).
3. Open the image from the repo, check its measured fs-verity digest equals the
   cmdline digest (composefs-rs `open_digest`), mount EROFS detached.
4. Scrub upper: remove upper entries whose names are in the image (they are sealed
   copies or runtime modifications of sealed paths: the image wins), keep
   whiteouts for names in the image (GC'd after the seal), drop stale whiteouts, and
   clear overlay xattrs on the upper root. This is what guarantees nothing in the
   writable layer can shadow a sealed path.
5. Mount overlay on /sysroot/nix/store (verity=require). Failure = emergency shell.

## Known constraints found so far
- Nix GC deletes sealed paths via overlay whiteouts; disk is freed only after the
  next seal + object GC.
- `auto-optimise-store` / `nix-store --optimise` would copy-up lower files into the
  upper layer: must be off. composefs already dedups content by digest.
- Nix's `makeStoreWritable` only remounts the bind mount rw; fine with an overlay.
