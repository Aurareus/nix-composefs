# Common configuration for the composefs-store systems built here (VM test
# images and the real-hardware USB image).
{ lib, pkgs, ... }:
{
  nixcfs.enable = lib.mkDefault true;

  boot.initrd.systemd.enable = true;
  boot.loader.systemd-boot.enable = true;
  # Images are written to removable/new disks: don't depend on NVRAM entries.
  boot.loader.efi.canTouchEfiVariables = false;
  boot.loader.systemd-boot.graceful = true;

  fileSystems."/" = {
    device = "/dev/disk/by-label/nixos";
    fsType = lib.mkDefault "btrfs";
  };
  fileSystems."/boot" = {
    device = "/dev/disk/by-label/ESP";
    fsType = "vfat";
    options = [ "umask=0077" ];
  };

  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  # Sandboxed builds from inside the system are part of the tests.
  nix.settings.sandbox = true;

  environment.systemPackages = with pkgs; [ fsverity-utils jq btrfs-progs e2fsprogs ];

  system.stateVersion = "26.05";
}
