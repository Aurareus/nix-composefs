# Real-hardware test image: Plasma desktop on a USB stick, booted on a
# Surface Pro 4 (Skylake, Marvell 88W8897 Wi-Fi). Generic x86_64 UEFI otherwise.
{ lib, pkgs, modulesPath, ... }:
{
  imports = [
    ./base.nix
    (modulesPath + "/profiles/all-hardware.nix")
  ];

  networking.hostName = lib.mkDefault "nixcfs-hw";
  networking.networkmanager.enable = true;
  hardware.enableRedistributableFirmware = true; # mwifiex firmware, i915 DMC
  hardware.graphics.enable = true;

  services.desktopManager.plasma6.enable = true;
  services.displayManager.sddm.enable = true;
  services.displayManager.sddm.wayland.enable = true;
  services.pipewire = { enable = true; pulse.enable = true; };

  # Grow the root partition and filesystem to fill the stick on first boot.
  boot.growPartition = true;
  fileSystems."/".autoResize = true;

  # Booting from USB: make sure the stick is found early.
  boot.initrd.availableKernelModules = [ "xhci_pci" "usb_storage" "uas" "sd_mod" "nvme" ];
  boot.kernelParams = [ "systemd.show_status=1" ];

  # Test account (this is a throw-away test image).
  users.users.nixcfs = {
    isNormalUser = true;
    extraGroups = [ "wheel" "networkmanager" ];
    initialPassword = "nixcfs";
  };
  security.sudo.wheelNeedsPassword = false;

  environment.systemPackages = with pkgs; [
    firefox kdePackages.kate kdePackages.konsole
    pciutils usbutils hdparm sysstat time
    (pkgs.callPackage ../../bench/package.nix { })
    (pkgs.callPackage ../../bench/hwtest-package.nix { })
  ];

  environment.etc."nixcfs-generation".text = lib.mkDefault "1\n";

  time.timeZone = "UTC";
  i18n.defaultLocale = "en_US.UTF-8";
}
