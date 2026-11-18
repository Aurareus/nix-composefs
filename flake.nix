{
  description = "NixOS with a composefs-backed /nix/store";

  inputs = {
    # Pinned: nixos-26.05 channel revision as of 2026-11-18.
    nixpkgs.url = "github:NixOS/nixpkgs/f68407657ad294471eb4b2e65d193df26edf729f";
    # Pinned: composefs-rs main as of 2026-11-12.
    composefs-rs = {
      url = "github:containers/composefs-rs/424c820c5e5237fef9e73a7446415108ab3eb7f9";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, composefs-rs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      nixosConfigurations.lab-minimal = nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [ ./nixos/lab-minimal.nix ];
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc clippy rustfmt pkg-config openssl sqlite e2fsprogs fsverity-utils erofs-utils qemu_kvm python3 jq ];
      };
    };
}
