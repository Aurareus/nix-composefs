# Generation 1 of the test system.
{ pkgs, ... }: {
  environment.systemPackages = [ pkgs.cowsay ];
  environment.etc."nixcfs-generation".text = "1\n";
}
