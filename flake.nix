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
      lib = nixpkgs.lib;
      nixcfs = pkgs.callPackage ./nix/nixcfs-package.nix { composefs-rs-src = composefs-rs; };
      nixcfsModule = { lib, ... }: {
        imports = [ ./nixos/modules/nixcfs.nix ];
        nixcfs.package = lib.mkDefault nixcfs;
      };
      mkSystem = modules: lib.nixosSystem { inherit system; modules = [ nixcfsModule ] ++ modules; };
      mkImage = args: import ./nixos/lib/disk-image.nix ({ inherit pkgs; } // args);
    in {
      packages.${system} = {
        inherit nixcfs;
        default = nixcfs;
        vm-image = mkImage { config = self.nixosConfigurations.vm-gen1.config; };
      };

      nixosModules.default = nixcfsModule;

      nixosConfigurations = {
        lab-minimal = lib.nixosSystem { inherit system; modules = [ ./nixos/lab-minimal.nix ]; };
        vm-gen1 = mkSystem [ ./nixos/configs/vm-test.nix ./tests/gen1.nix ];
        vm-gen2 = mkSystem [ ./nixos/configs/vm-test.nix ./tests/gen2.nix ];
      };

      checks.${system} = import ./tests { inherit pkgs self mkImage; };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc clippy rustfmt pkg-config openssl sqlite e2fsprogs fsverity-utils erofs-utils qemu_kvm python3 jq btrfs-progs ];
      };
    };
}
