# Minimal NixOS system used for the first composefs mount experiments.
{ lib, modulesPath, ... }: {
  imports = [ (modulesPath + "/profiles/minimal.nix") ];
  boot.loader.grub.enable = false;
  fileSystems."/" = { device = "/dev/disk/by-label/nixos"; fsType = "btrfs"; };
  system.stateVersion = "26.05";
}
