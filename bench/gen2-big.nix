# Benchmark "update": a few GB of new store paths (office/graphics apps).
{ pkgs, lib, ... }: {
  environment.systemPackages = with pkgs; [ libreoffice-qt6-fresh gimp inkscape ];
  environment.etc."nixcfs-generation".text = lib.mkForce "2\n";
}
