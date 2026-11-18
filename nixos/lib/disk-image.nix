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
      nixos-enter --root /mnt -- ${lib.getExe nixcfs} init
      NIXOS_INSTALL_BOOTLOADER=1 nixos-enter --root /mnt -- \
        /nix/var/nix/profiles/system/bin/switch-to-configuration boot
      mkdir -p $out/nix-support
      nixos-enter --root /mnt -- ${lib.getExe nixcfs} status | tee $out/status.json
      cat /mnt/boot/loader/entries/*.conf | tee $out/entries.txt
      grep -q nixcfs.image= $out/entries.txt

      umount /mnt/boot
      umount /mnt
    ''
)
