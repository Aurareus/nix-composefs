# Real-hardware test (Surface Pro 4, round 1)

Images (built 2026-11-18, commit tagged `hw-round-1`), on the build host:

| Image | Path | sha256 |
|---|---|---|
| A: composefs store (the thing under test) | `~/composefs-nixos/result-hw-image/disk.raw` (15.3 GB) | `a000c6a4f68e16ecf2581c1fc3a3e8d4bc5577ca2e624fbc2fc7fdf226a3320a` |
| B: same system, ordinary store (comparison; optional) | `~/composefs-nixos/result-hw-image-plain/disk.raw` (15.3 GB) | `43c51cf947c0c333a0d0939a105cb7e9395ad348373819e891b135cfce77a25b` |

Both are KDE Plasma 6 on NixOS 26.05, btrfs root, systemd-boot, UEFI. The root
partition grows to fill the stick on first boot. Login: user `nixcfs`, password
`nixcfs` (passwordless sudo). This is a throwaway test account.

Already verified in QEMU (UEFI, image attached as a USB stick, Skylake CPU
model): boots to the SDDM greeter and Plasma, and all three test stages below pass.

## 1. Write image A to the USB stick (on the build host)

Plug the 128 GB stick into the build host, then:

```sh
lsblk -o NAME,SIZE,MODEL,TRAN,MOUNTPOINTS
```

Find the stick: `TRAN` = `usb`, size about 115–128G. Below it is called
`/dev/sdX`. **Double-check the name: everything on it is overwritten.**

```sh
sudo umount /dev/sdX?* 2>/dev/null
sudo dd if=$HOME/composefs-nixos/result-hw-image/disk.raw of=/dev/sdX bs=4M conv=fsync status=progress
sync
```

(Expect 5–15 minutes depending on the stick.) Then unplug it.

## 2. Boot the Surface Pro 4 from it

1. Surface off. Plug the stick into the USB-A port.
2. Press and hold **Volume Down**, press and release **Power**, keep holding
   Volume Down until the Surface logo is replaced by the boot menu/spinner.
   (Secure Boot is already off on this machine, which this image needs.)
3. The systemd-boot menu shows "NixOS" for 5 s; let it boot the default.
4. Expected: kernel boot, then the SDDM login screen. The first boot takes
   longer (it grows the partition and cleans up the install-time store).
   Notes: the Type Cover keyboard and touchpad work; the **touchscreen will not**
   (it needs the out-of-tree linux-surface IPTS driver, not part of this test).

If it does **not** reach the login screen, take a photo of the screen and send
it. If it stops in an emergency shell with a nixcfs error, that is the
integrity check refusing to boot. Send a photo of that too.

## 3. Run the test (three stages, two reboots)

Log in (`nixcfs` / `nixcfs`). Optional: connect to Wi-Fi from the network
icon in the panel and note whether it works (hardware sanity only; the test
itself needs no network). Open **Konsole** (Ctrl+Alt+T) and run:

```sh
sudo nixcfs-hwtest 1
```

This takes roughly 10–20 minutes: boot/disk/read-performance measurements, a
non-destructive tamper check, then a rebuild to "generation 2" from a bundle on
the stick. Keep the machine idle and on AC. If the screen locks, unlock with
the same password. At the end it must print `stage 1 result: ALL PASS`.

**Reboot** (just let the default entry boot), log in, open Konsole:

```sh
sudo nixcfs-hwtest 2
```

(~5 minutes; it verifies all store contents). It must end with `ALL PASS`.
It sets the *next* boot to generation 1 (rollback through the boot menu).

**Reboot again**, without touching the boot menu, log in, Konsole:

```sh
sudo nixcfs-hwtest 3
```

Must end with `stage 3 result: ALL PASS`. Then **shut down**.

## 4. Send back the results

Everything is logged to the stick's FAT partition (`ESP`), folder
`nixcfs-reports/`. Easiest: plug the stick into the build host and tell me.
I'll copy the reports off read-only. Please also tell me:

- whether each stage printed ALL PASS (or paste the FAIL lines);
- whether Wi-Fi worked;
- anything that looked wrong (photos are fine).

## 5. Optional round B (comparison numbers on real hardware)

Only after I've copied the round-A reports off the stick (writing image B
erases them): write image B the same way
(`if=$HOME/composefs-nixos/result-hw-image-plain/disk.raw`), boot it, log in,
and run only:

```sh
sudo nixcfs-bench
```

(~10 minutes, keep idle), shut down, and plug the stick back into the host.
