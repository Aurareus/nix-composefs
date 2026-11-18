# Generation 2: drops cowsay (so GC has something to remove), adds hello.
{ pkgs, ... }: {
  environment.systemPackages = [ pkgs.hello ];
  environment.etc."nixcfs-generation".text = "2\n";
}
