{ writeShellApplication, coreutils, findutils, util-linux, bc, systemd, gnugrep, gawk, btrfs-progs, fsverity-utils, nix }:
writeShellApplication {
  name = "nixcfs-bench";
  runtimeInputs = [ coreutils findutils util-linux bc systemd gnugrep gawk btrfs-progs fsverity-utils nix ];
  checkPhase = "";
  text = builtins.readFile ./nixcfs-bench.sh;
}
