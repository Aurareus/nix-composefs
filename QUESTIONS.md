# Open questions for the user

Asked 2026-11-18T02:00Z. None of these block current work; the default in **bold**
is what I am doing until you say otherwise.

## Q1. When does a newly built generation become verified? (security trade-off)
Design (details in DESIGN-NOTES.md): `/nix/store` = overlayfs with a composefs
(EROFS + fs-verity object store) lower layer holding every *sealed* store path, plus
a writable upper layer where Nix puts paths it adds at runtime. "Sealing" moves upper
content into the verified image.

- **A (default): seal at `nixos-rebuild boot/switch` time, take effect at next boot.**
  After `nixos-rebuild switch` the new generation *runs* from the unverified upper
  layer until you reboot; after the reboot everything sealed is verified. Simple,
  robust, no live remounting of /nix/store.
- B: additionally swap the running /nix/store to the new image live after each switch
  (mount-beneath + lazy unmount). Verified immediately, but more moving parts
  (nix-daemon restart, mount propagation into service namespaces). I'd try this only
  after A is solid, as an option.

## Q2. What happens if the store image fails verification at boot?
- **Hard fail into the initrd emergency shell (no unverified fallback).** Other boot
  entries remain selectable. This is the only choice that keeps "tampered content is
  refused" true.
- Alternative: fall back to an unverified mount and warn loudly. Boots, but defeats the point.

## Q3. Filesystem for /nix on the real machine
fs-verity needs ext4 (with the `verity` feature) or btrfs (or f2fs). XFS does not
support fs-verity (as far as I've checked so far). **I test ext4 first and also
support btrfs.** If you already know you want btrfs (e.g. for snapshots), tell me
and I'll make it the primary test target.

## Q4. nixpkgs branch
**nixos-26.05 (stable), pinned at f68407657ad2.** Say if your daily driver will track
nixos-unstable instead.

## Q5. Desktop environment (for realistic measurements and the hardware test image)
**GNOME** unless you tell me otherwise (Plasma, a tiling WM, ...).

## Q6. The real-hardware test machine
Please send: CPU/GPU (any NVIDIA?), whether it boots UEFI, whether Secure Boot is
enabled (my image's bootloader is unsigned, so it would need to be disabled for
testing, or you'd use a machine where it is off), size of a USB stick you can
overwrite (>= 32 GB preferred), and whether it has wired Ethernet or needs Wi-Fi.
