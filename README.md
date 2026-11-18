# composefs-nixos: NixOS with its /nix/store served through composefs

`/nix/store` is an overlayfs whose lower layer is a composefs image (an EROFS
metadata image over a content-addressed, fs-verity protected object store) and
whose upper layer takes the paths Nix adds at runtime. With `verity=require`,
the kernel checks every sealed store file against the digest recorded in the
image. The image's own digest is on the kernel command line, so a modified store
file is refused (EIO) instead of being served.

**Start with [RESULTS.md](RESULTS.md)**: design, limitations, security
properties and measurements.

## Layout of this repository

| Path | What |
|---|---|
| `nixcfs/` | The tool (Rust, on composefs-rs): `seal`, `boot`, `update-boot`, `gc`, `status`, `object-of` |
| `nixos/modules/nixcfs.nix` | NixOS module: initrd mount service, systemd-boot hook, cleanup/GC services |
| `nixos/lib/disk-image.nix` | Builds bootable raw disk images (sealing happens inside a build VM) |
| `nixos/configs/` | `base.nix` (shared), `vm-test.nix` (tests), `hw-plasma.nix` (USB/hardware image) |
| `tests/` | NixOS VM tests (`nix flake check` or `nix build .#checks.x86_64-linux.<name>`) |
| `bench/` | `nixcfs-bench` measurement script and the VM comparison benchmark |
| `patches/` | Patches to upstream projects, one per file, with rationale in the commit message |
| `TIMELOG.md`, `QUESTIONS.md`, `HOST-SETUP.md`, `DESIGN-NOTES.md` | Project bookkeeping |

## Using it on a machine

Requirements: UEFI with systemd-boot, the systemd initrd, and a root (or
`/nix`) filesystem with fs-verity support: **btrfs**, or ext4 created with
`-O verity` (`tune2fs -O verity` on an existing one). XFS does not work.

```nix
# flake.nix of your system
inputs.composefs-nixos.url = "git+ssh://git@github.com/Aurareus/nix-composefs";
# ...
nixosConfigurations.mymachine = nixpkgs.lib.nixosSystem {
  modules = [
    composefs-nixos.nixosModules.default
    {
      nixcfs.enable = true;
      boot.initrd.systemd.enable = true;
      boot.loader.systemd-boot.enable = true;
    }
    ./configuration.nix
  ];
};
```

Then run `nixos-rebuild boot` and reboot. The first bootloader install creates
the object store under `/nix/.cfs` and seals the existing (plain) store. On the
next boot the initrd mounts the verified store and moves the old plain store
aside. A background service then deletes it; file data is shared with the
object store (reflinks on btrfs, hardlinks on ext4), so this frees only
directory metadata.

Day to day nothing changes: `nixos-rebuild switch|boot`, `nix build`,
`nix-collect-garbage -d` all work. Things to know:

- A generation you `switch` to runs from the writable layer until the next
  reboot. Sealing happens at every bootloader install, and the reboot is what
  makes the new paths verified. `nixcfs status` shows the state.
- Boot entries of generations from before nixcfs was enabled are removed:
  their initrd cannot mount the new store. Do not `switch --rollback` past the
  conversion point either. That generation's bootloader installer would write
  entries without the image digest, and nothing would boot until you run a
  newer generation's `bin/switch-to-configuration boot`.
- `nix.settings.auto-optimise-store` must stay off; the module asserts this.
  Composefs already deduplicates file contents.
- After garbage collection, disk space held by paths that were in the booted
  image is returned after the next `nixos-rebuild boot` (re-seal) plus reboot.
  The `nix-gc` service re-seals automatically.

## Commands

```
nixcfs status                  # booted/latest image, unsealed paths, runtime modifications
nixcfs update-boot             # seal + stamp boot entries + GC (what the bootloader hook runs)
nixcfs object-of /nix/store/…  # which object file backs a store file
nixcfs gc --esp /boot          # drop images/objects nothing references
```

## Running the tests

```
nix build .#checks.x86_64-linux.boot        # also: tamper tamper-ext4 lifecycle nix-ops robustness convert
nix build .#benchmarks.x86_64-linux.desktop # comparison benchmark (hours)
```

The tests need KVM in the Nix sandbox (see HOST-SETUP.md).
