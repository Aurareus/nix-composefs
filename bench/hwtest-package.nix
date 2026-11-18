{ writeShellApplication, coreutils, gnugrep, bc, systemd, nix }:
writeShellApplication {
  name = "nixcfs-hwtest";
  runtimeInputs = [ coreutils gnugrep bc systemd nix ];
  checkPhase = "";
  text = builtins.readFile ./nixcfs-hwtest.sh;
}
