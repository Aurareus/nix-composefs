# Host setup (Ubuntu 26.04.1 build/test host)

Everything done to the host, in order, so it can be reproduced.

1. Nix 2.35.6, multi-user, from the pinned installer
   `https://releases.nixos.org/nix/nix-2.35.6/install`
   (sha256 6f7bf206efb54cd4941fb274930dbc74fdfc16fcb06757ef261c8d7d69cfd60e,
   copy in logs/nix-install-2.35.6.sh): `sh install --daemon --yes`.
2. /etc/nix/nix.conf additions:
   ```
   experimental-features = nix-command flakes
   system-features = kvm nixos-test benchmark big-parallel
   max-jobs = 4
   cores = 8
   trusted-users = root user
   ```
3. /dev/kvm usable from the Nix build sandbox (the sandbox drops supplementary
   groups, so group membership is not enough):
   `/etc/udev/rules.d/65-kvm.rules`:
   `KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"`
   (nixbld users were also added to group kvm; harmless, not sufficient).
4. Scratch filesystems for experiments: loop-mounted ext4 (`-O verity`) and btrfs
   images under ~/scratch (not needed for the tests, which run in VMs).
5. Ubuntu 26.04 mounts /tmp as a 16 GB tmpfs, and Nix builds there by default:
   VM disk overlays then live in RAM and QEMU `cache=none` (O_DIRECT) fails.
   Added `build-dir = /nix/var/nix/builds` to /etc/nix/nix.conf (on the NVMe).
