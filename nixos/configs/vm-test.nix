# VM variant used by the automated tests: test-driver backdoor, serial console.
{ modulesPath, lib, pkgs, ... }:
{
  imports = [
    ./base.nix
    (modulesPath + "/profiles/qemu-guest.nix")
    (modulesPath + "/testing/test-instrumentation.nix")
  ];
  networking.hostName = "cfsvm";
  # test-instrumentation turns off documentation etc.; keep the image small.
  documentation.enable = false;
  boot.loader.timeout = lib.mkForce 1;
}
