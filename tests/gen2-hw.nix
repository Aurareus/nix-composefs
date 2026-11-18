# Generation 2 of the hardware test image.
{ pkgs, lib, ... }: {
  environment.etc."nixcfs-generation".text = lib.mkForce "2\n";
  environment.systemPackages = [ pkgs.hello ];
}
