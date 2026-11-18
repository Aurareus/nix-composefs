# Results: NixOS with a composefs-backed /nix/store

Status: DRAFT (measurements and hardware results pending)

## 1. Summary

NixOS boots with `/nix/store` served through composefs (composefs-rs 0.10,
EROFS + overlayfs + fs-verity, `verity=require`). Rebuilds, reboots, boot-menu
and runtime rollbacks, Nix builds, adding paths and garbage collection all work,
on btrfs and on ext4. Automated VM tests cover each of these, plus seven kinds
of tampering, all of which are refused.

The goal is sound and the core of it is met. Two things are worth knowing up front:

1. **Verification starts at boot, not at build.** Paths Nix adds while the
   system runs live in a writable layer and are unverified until they are
   *sealed* (at every `nixos-rebuild boot|switch`) **and** the machine
   reboots into the new image. After `nixos-rebuild switch` the new generation
   therefore runs unverified until the next reboot (you chose this, Q1).
2. **Without Secure Boot the trust anchor is unprotected.** The image digest
   lives on the kernel command line in the boot entry on the ESP. Anyone who
   can write the ESP (offline, or root at runtime) can point it at a different
   image they built. What this delivers is enforcement that the store matches
   the boot entry, on every read from disk. Making that unforgeable needs a
   signed UKI (Secure Boot); this design is built so that adding it is a
   bootloader change, not a redesign (section 8).

## 2. Design

```
 kernel cmdline:  nixcfs.image=<sha256 fs-verity digest>          (trust anchor)
                         │
 initrd (systemd)  nixcfs boot ── open /nix/.cfs/images/<digest>, measure its
                         │        fs-verity digest, must equal the cmdline
                         │     ── scrub upper layer (see below)
                         ▼
 /nix/store = overlayfs( upperdir  = /nix/.cfs-rw/upper      writable, unverified
                         lowerdir+ = EROFS image (detached)  metadata: names, modes,
                                                             symlinks, small files,
                                                             per-file digest + redirect
                         datadir+  = /nix/.cfs/objects       file data, by digest,
                                                             fs-verity enabled
                         metacopy=on, redirect_dir=on, verity=require )
```

* **Object store** (`/nix/.cfs`, a composefs-rs repository): one file per
  distinct content, named by its fs-verity digest, with fs-verity enabled.
  Files of 64 bytes or less are stored inside the image. Identical files
  across all store paths and generations are stored once.
* **Image**: one EROFS image holds *every valid store path* at seal time, not
  just one system closure. Every boot entry uses the latest image, so any
  generation in the boot menu, and `switch --rollback` at runtime, finds all
  its paths verified.
* **Sealing** (`nixcfs seal`, run from the systemd-boot install hook, i.e. by
  every `nixos-rebuild boot|switch`, and after `nix-gc`):
  1. take Nix's GC lock shared (GC waits, builds don't);
  2. read the valid paths from the Nix DB;
  3. reuse the subtrees of paths already in the previous image, without
     reading their data;
  4. scan new paths from the writable layer. File data enters the object store
     without copying: reflink on btrfs, hardlink plus in-place fs-verity on ext4;
  5. compute each new path's NAR hash *from the object store* (that is, through
     fs-verity) and compare it with the Nix DB. A mismatch aborts the seal;
  6. check that every referenced object exists, write the image, fsync;
  7. stamp `nixcfs.image=` on every boot entry, drop entries whose system is
     not in the image or predates nixcfs, and GC images/objects that nothing
     references (latest, entries, or the booted image).
* **Boot** (`nixcfs boot` in the initrd): verify the image as above. Then
  *scrub* the upper layer: any upper entry whose name is a store path in the
  image is moved to a trash directory (only whiteouts, i.e. GC'd paths, may
  remain). Stray whiteouts and overlay xattrs on the upper root are removed.
  Then mount. There is no fallback: if anything fails, the initrd stops at the
  emergency target (Q2). The scrub is what makes the writable layer safe:
  nothing in it can shadow a sealed path, so every lookup under a sealed path
  resolves only in the verified layers.
* **GC**: Nix's GC works unchanged. Deleting a sealed path creates an overlay
  whiteout. The next seal leaves the path out, and the following boot drops the
  whiteout. Objects only the old image used are deleted once nothing references
  that image (after the reboot).

### Why this design, and what I rejected

| Alternative | Why not |
|---|---|
| **One image per generation** (bootc/ostree style: image = that system's closure) | Runtime rollback and other generations' paths would be unverified or missing; deduplicating the upper layer at boot then breaks rollback in one direction or the other. One image of all valid paths avoids this, and costs little because images hold only metadata. |
| **Read-only store, no writable layer** (dm-verity/erofs image of the whole store, appliance style) | Nix could not build, add or GC at runtime, which is a hard requirement. |
| **Live re-mount of /nix/store after each switch** (mount-beneath + lazy unmount) | Would verify new generations without a reboot. Rejected for now (your Q1 answer): it needs nix-daemon quiescing, mount propagation into every service namespace, and an upper-layer swap. Listed under next steps. |
| **Nix `local-overlay` store** (experimental) | Designed for a separate lower store with its own DB (e.g. shared over NFS). Ours is a snapshot of our own store, so one DB plus plain overlay semantics is simpler and needs no experimental Nix features. |
| **IMA/EVM appraisal or IPE** | Per-file signatures/policy rather than a content-addressed snapshot; needs keys and signing on every build; does not give "this file equals what the image says". IPE could later *complement* this (e.g. only execute from verified files). |
| **fs-verity on store files without composefs** | Makes files immutable, but nothing holds the expected digest, so a replaced file (with its own verity) is accepted. composefs's image is that expected-digest table, enforced by overlayfs. |
| **Two stacked overlays** (plain composefs mount as lower of a second writable overlay) | Works, but doubles overlay lookups on every path. A single overlay with upper + lower + data-only layer works on 6.18 and was chosen. |
| **sha512 digests** | Measured: slower than sha256 both on SHA-NI CPUs and on Skylake (section 6). |
| **`cfsctl create-image`** as the sealing tool | It copies all data and re-reads everything each time; the store needs incremental, zero-copy sealing with NAR checks. `nixcfs` uses composefs-rs as a library instead. |

## 3. Limitations

* **systemd-boot only** (UEFI). The digest is stamped into BLS entries. GRUB
  and Limine would need their own hook.
* **Reboot required for verification** of newly built paths (above). Until
  then `nixcfs status` lists them as unsealed.
* **Disk space of deleted sealed paths** returns only after re-seal + reboot +
  the post-boot GC. The `nix-gc` service re-seals automatically; a manual
  `nix-collect-garbage` should be followed by `nixos-rebuild boot`.
* **Root on the running system can still change what is served** until the next
  boot: remount the store bind rw and modify a file, which overlayfs copies up
  into the writable layer. `nixcfs status` reports such paths
  (`shadowing_sealed_paths`) and the next boot reverts them. Root can also
  simply unmount /nix/store. See section 4.
* **Pre-conversion generations** lose their boot entries when converting an
  existing system; don't `switch --rollback` past the conversion (README).
* **`auto-optimise-store` must be off**: hard-linking lower-layer files copies
  them up. Composefs deduplicates by content anyway.
* **A non-reproducible path that is GC'd and rebuilt between a seal and a
  reboot** comes back as the sealed (older) build at the next boot. That is
  valid content for the same derivation, but `nix-store --verify` would then
  flag a NAR hash mismatch until the next seal. This is a rare corner.
* **Memory/time of sealing** scales with the store's metadata (the whole image
  is rebuilt each time from the previous one plus new paths; data of existing
  paths is not touched). See measurements.
* **Single filesystem**: the object store, writable layer and image must be on
  the same fs-verity-capable filesystem (btrfs or ext4 `-O verity`). XFS has no
  fs-verity.

## 4. Security properties

### What the kernel enforces

For every regular file of a sealed store path:

* **At open**, overlayfs compares the backing object's fs-verity digest with the
  digest recorded for that file in the image. A missing digest or a different
  one gives EIO. The comparison is cached per inode, so it happens when the
  file's inode is first instantiated, not on every `open()`.
* **At every read from storage**, fs-verity checks each 4 KiB block against the
  object's Merkle tree. Pages already in the page cache are not re-checked. A
  block modified on disk while its page is cached is detected once the page is
  evicted and read again; the tests drop caches to show this.
* Directory structure, names, modes, ownership, symlink targets and files of 64
  bytes or less live in the EROFS image itself. The image file has fs-verity
  too, and its digest is checked against the kernel command line before it is
  mounted. That covers all store metadata: no symlink, permission or directory
  entry of a sealed path can be changed either.

So "the kernel checks every store file on every read" holds in this precise
form: every read *from disk* is checked, as is the first open of each file.

### Threat by threat (all "tested" items are in `tests/tamper.py`, run on btrfs and ext4)

| Attacker / event | Result | Evidence |
|---|---|---|
| Disk corruption or bit rot in store file data | Refused (EIO), never served | tested: raw block write to the device under an object. ext4: fs-verity `FILE CORRUPTED`; btrfs: caught by btrfs data checksums first (also EIO), fs-verity as second line |
| Offline: replace an object with modified content, fs-verity enabled on it | Refused at open (`wrong fs-verity digest`) | tested |
| Offline: replace an object with modified content, no fs-verity | Refused at open (`has no fs-verity digest`) | tested |
| Offline: modify the image (metadata) | Boot refused: initrd stops, no fallback | tested (`Expected digest … but found …`, emergency target) |
| Offline: plant a replacement for a sealed path in the writable layer | Removed at boot by the upper-layer scrub; the verified copy is served | tested (`boot-report.json: removed_shadowing`) |
| Offline: plant whiteouts / opaque markers to hide sealed paths | Hiding only (availability). Opaque markers on the upper root are stripped; whiteouts of sealed names are kept, because GC uses them | partly tested |
| Offline: modify paths added since the last seal (writable layer) | **Not protected**: served as is. The next seal recomputes NAR hashes and refuses to seal content that differs from the Nix DB (tested in `tests/robustness.py`), but an attacker who also edits the DB can get content sealed | NAR check tested |
| Offline: edit the boot entry / kernel / initrd on the ESP | **Not protected** without Secure Boot: they can name another image they sealed themselves, or boot an initrd that skips verification | n/a |
| Root on the running system | **Not protected** until reboot. Root can remount the store bind mount rw and modify files (copy-up into the writable layer; tested: `nixcfs status` reports it, next boot reverts it), unmount /nix/store, or seal its own content and stamp it into the boot entries | tested (runtime modification) |
| Unprivileged users | Store read-only as on stock NixOS | n/a |
| Rest of the root filesystem (`/etc` state, `/var`, `/home`, `/nix/var`) | **Not covered**: only /nix/store is. NixOS's `/etc` is mostly symlinks into the store, so mostly covered; mutable files like `/etc/shadow` are not | n/a |

### Chain of custody of store content

For a substituted path: Nix checks the substituter's signature and the NAR hash
when downloading, and records the NAR hash in the DB. `nixcfs seal` recomputes
the NAR hash of the content it actually imported (read back through fs-verity)
and refuses to seal on a mismatch. From then on, the image digest pins the
content, and the image digest sits in the boot entry. The weak links are the
Nix DB between download and seal, and the boot entry itself.

### What would complete it

A signed UKI per boot entry with `nixcfs.image=` in its embedded command line
(lanzaboote-style Secure Boot) makes the boot-entry link unforgeable offline.
With that, an offline attacker can no longer get modified sealed content served.
Runtime root remains out of scope, as for any dm-verity/composefs system. IPE
could additionally restrict execution to verified files.

## 5. Testing

TODO

## 6. Measurements

TODO

## 7. Patches for upstream

TODO

## 8. What next

TODO

## 9. Pinned revisions

TODO
