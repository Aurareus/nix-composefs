# Time log (UTC)

## Working stretches
- 2026-11-18T01:47:48Z — start of stretch 1 (read brief, survey host; first command run at this time)
- 2026-11-18T05:48:36Z — end of stretch 1 (only real-hardware results outstanding; waiting on user)

## Blocked on user
(none so far: questions Q1–Q6 asked 2026-11-18T02:00Z were non-blocking; answered ~02:25Z)
- 2026-11-18T05:09:05Z — START waiting on user: real-hardware test round 1 (HARDWARE-TEST.md). Not
  blocking: VM benchmarks and the write-up continue in parallel.

## Estimates
- 2026-11-18T01:57:23Z — initial estimates, recorded before writing code (after surveying composefs-rs
  main, nixpkgs nixos-26.05, Nix 2.34 GC/mount code, host kernel features):
  - (a) first NixOS system closure mounted through composefs with verity enforced:
    ~3 h of working time from the start of stretch 1, i.e. around 2026-11-18T05:00Z.
  - (b) "done" as defined in the brief: ~35 h of working time; ~4 calendar days
    including real-hardware round trips and overnight answer latency.

## Milestones
- 2026-11-18T02:19:57Z — (a) first NixOS system closure (26.05 minimal toplevel, 505 paths,
  1.15 GB) mounted through composefs with verity=require, on btrfs, sealed by nixcfs
  (NAR hashes all match the Nix DB; `nix-store --verify --check-contents` passes through
  the mount). ~32 min after start vs. the 3 h estimate: composefs-rs already had
  everything needed (upper-layer mounts, reflink/hardlink import) bar one panic fix.
- 2026-11-18T02:40:13Z — first boot of NixOS from the composefs store (VM: QEMU/KVM,
  OVMF UEFI, systemd-boot, btrfs root; store image verified against the digest on
  the kernel command line, verity=require). Real hardware: pending.
- ~2026-11-18T03:00Z (VM; approximate, from the test log of the run in which it first
  happened) — first rebuild (gen 1 → gen 2, switch), boot into gen 2 (verified), and
  rollback to gen 1 through the boot menu. That run failed later only at
  `nixos-rebuild switch --rollback` (test bug: offline re-exec).
- 2026-11-18T03:43:42Z — full lifecycle test green (rebuild, reboot, boot-menu rollback,
  runtime `nixos-rebuild switch --rollback`, reboot into the rolled-back default).
- 2026-11-18T05:48:36Z — full test suite green on the final code (boot, tamper, tamper-ext4, lifecycle,
  nix-ops incl. nix-gc re-seal hook, robustness, convert); VM desktop benchmarks done.
  Everything except real hardware is complete; RESULTS.md awaits hardware numbers.
