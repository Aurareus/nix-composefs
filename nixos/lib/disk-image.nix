# Build a raw GPT disk image (ESP + btrfs or ext4 root) holding a NixOS system
# whose store is already sealed into a composefs image.  The whole install runs
# inside a VM, because sealing needs a real kernel filesystem with fs-verity.
{
  pkgs,
  lib ? pkgs.lib,
  config, # evaluated NixOS config (nixosSystem ...).config
  fsType ? "btrfs",
  espSizeMiB ? 1024,
  extraSpaceMiB ? 2048,
  name ? "nixcfs-disk",
  # Extra systems to ship as a local binary cache (only their paths that are
  # not already in the image), for offline rebuild tests.
  cacheSystems ? [ ],
}:
let
  toplevel = config.system.build.toplevel;
  closure = pkgs.closureInfo { rootPaths = [ toplevel ]; };
  nixcfs = config.nixcfs.package;
  mkfs =
    {
      btrfs = "mkfs.btrfs -q -L nixos /dev/vda2";
      ext4 = "mkfs.ext4 -q -O verity -L nixos /dev/vda2";
    }
    .${fsType};
in
pkgs.vmTools.runInLinuxVM (
  pkgs.runCommand name
    {
      memSize = 4096;
      QEMU_OPTS = "-smp 4";
      preVM = ''
        mkdir -p $out
        size=$(( $(cat ${closure}/total-nar-size) / 1048576 * 13 / 10 + ${toString (espSizeMiB + extraSpaceMiB)} ))
        diskImage=$out/disk.raw
        ${pkgs.qemu-utils}/bin/qemu-img create -f raw $diskImage ''${size}M
      '';
      postVM = ''
        echo ${toplevel} > $out/toplevel
      '';
      nativeBuildInputs = with pkgs; [
        util-linux
        dosfstools
        btrfs-progs
        e2fsprogs
        config.system.build.nixos-install
        nixos-enter
        nix
        jq
      ];
    }
    ''
      set -euo pipefail
      sfdisk --quiet /dev/vda <<EOF
      label: gpt
      size=${toString espSizeMiB}M, type=uefi, name=ESP
      type=linux, name=nixos
      EOF
      mkfs.vfat -F 32 -n ESP /dev/vda1
      ${mkfs}
      mkdir -p /mnt
      mount /dev/vda2 /mnt
      mkdir -p /mnt/boot
      mount -o umask=0077 /dev/vda1 /mnt/boot

      # Copy the closure into the image and register it.
      export NIX_STATE_DIR=$TMPDIR/state
      nix-store --load-db < ${closure}/registration
      nixos-install --root /mnt --system ${toplevel} --no-bootloader --no-root-passwd --no-channel-copy

      # Create the composefs repository; installing the bootloader then seals
      # the (still plain) store and stamps the image digest on the entries.
      # (With nixcfs enabled, installing the bootloader creates the object
      # store, seals the still-plain store and stamps the image digest.)
      NIXOS_INSTALL_BOOTLOADER=1 nixos-enter --root /mnt -- \
        /nix/var/nix/profiles/system/bin/switch-to-configuration boot
      ${lib.optionalString (cacheSystems != [ ]) ''
        mkdir -p /mnt/var/lib/nixcfs-hwtest/cache
        nix-store -qR ${toplevel} | sort > $TMPDIR/have
        nix-store -qR ${lib.concatStringsSep " " cacheSystems} | sort > $TMPDIR/want
        comm -13 $TMPDIR/have $TMPDIR/want > $TMPDIR/new
        echo "shipping $(wc -l < $TMPDIR/new) extra paths in the local cache"
        nix copy --extra-experimental-features nix-command --no-check-sigs --to file:///mnt/var/lib/nixcfs-hwtest/cache $(cat $TMPDIR/new)
      ''}
      cat /mnt/boot/loader/entries/*.conf | tee $out/entries.txt
      ${lib.optionalString config.nixcfs.enable ''
        nixos-enter --root /mnt -- ${lib.getExe nixcfs} status | tee $out/status.json
        grep -q nixcfs.image= $out/entries.txt
      ''}

      umount /mnt/boot
      umount /mnt
    ''
)
