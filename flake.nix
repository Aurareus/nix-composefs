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
      # Tells nixcfs-hwtest which system is "generation 2".  (Gen 2 has the
      # same pointer, computed without the pointer to avoid a cycle.)
      hwGen2 = mkSystem [ ./nixos/configs/hw-plasma.nix ./tests/gen2-hw.nix ];
      hwtestGen2Pointer = {
        # Only the path name: gen 2 must not be in gen 1's closure (it is shipped
        # separately, so that the hardware test exercises sealing new paths).
        environment.etc."nixcfs-hwtest/gen2".text =
          builtins.unsafeDiscardStringContext "${hwGen2.config.system.build.toplevel}";
      };
      mkImage = args: import ./nixos/lib/disk-image.nix ({ inherit pkgs; } // args);
    in {
      packages.${system} = {
        inherit nixcfs;
        default = nixcfs;
        vm-image = mkImage { config = self.nixosConfigurations.vm-gen1.config; };
        # Real-hardware USB images (raw, write with dd).
        hw-image = mkImage {
          config = self.nixosConfigurations.hw-plasma.config;
          name = "nixcfs-hw-plasma";
          cacheSystems = [ self.nixosConfigurations.hw-plasma-gen2.config.system.build.toplevel ];
        };
        hw-image-plain = mkImage { config = self.nixosConfigurations.hw-plasma-plain.config; name = "nixcfs-hw-plasma-plain"; };
      };

      nixosModules.default = nixcfsModule;

      nixosConfigurations = {
        lab-minimal = lib.nixosSystem { inherit system; modules = [ ./nixos/lab-minimal.nix ]; };
        vm-gen1 = mkSystem [ ./nixos/configs/vm-test.nix ./tests/gen1.nix ];
        vm-gen2 = mkSystem [ ./nixos/configs/vm-test.nix ./tests/gen2.nix ];
        hw-plasma = mkSystem [ ./nixos/configs/hw-plasma.nix hwtestGen2Pointer ];
        hw-plasma-gen2 = hwGen2;
        # Same system with an ordinary /nix/store, for comparison measurements.
        hw-plasma-plain = mkSystem [ ./nixos/configs/hw-plasma.nix { nixcfs.enable = false; networking.hostName = "nixcfs-hw-plain"; } ];
        # Benchmark systems: the Plasma desktop, in a VM, composefs vs ordinary store.
        bench-plasma = mkSystem [ ./nixos/configs/hw-plasma.nix ./nixos/configs/vm-test.nix { networking.hostName = "bench-cfs"; } ];
        bench-plasma-plain = mkSystem [ ./nixos/configs/hw-plasma.nix ./nixos/configs/vm-test.nix { nixcfs.enable = false; networking.hostName = "bench-plain"; } ];
        bench-plasma-gen2 = mkSystem [ ./nixos/configs/hw-plasma.nix ./nixos/configs/vm-test.nix ./bench/gen2-big.nix { networking.hostName = "bench-cfs"; } ];
        bench-plasma-plain-gen2 = mkSystem [ ./nixos/configs/hw-plasma.nix ./nixos/configs/vm-test.nix ./bench/gen2-big.nix { nixcfs.enable = false; networking.hostName = "bench-plain"; } ];
        bench-plasma-sha512-gen2 = mkSystem [ ./nixos/configs/hw-plasma.nix ./nixos/configs/vm-test.nix ./bench/gen2-big.nix { networking.hostName = "bench-cfs512"; nixcfs.algorithm = "fsverity-sha512-12"; } ];
        bench-plasma-sha512 = mkSystem [ ./nixos/configs/hw-plasma.nix ./nixos/configs/vm-test.nix { networking.hostName = "bench-cfs512"; nixcfs.algorithm = "fsverity-sha512-12"; } ];
        # Generation 1 with an ordinary store: starting point of the conversion test.
        vm-gen1-plain = mkSystem [ ./nixos/configs/vm-test.nix ./tests/gen1.nix { nixcfs.enable = false; } ];
        vm-gen1-ext4 = mkSystem [ ./nixos/configs/vm-test.nix ./tests/gen1.nix { fileSystems."/".fsType = "ext4"; } ];
      };

      checks.${system} = import ./tests { inherit pkgs self mkImage; };
      benchmarks.${system} = import ./bench { inherit pkgs self mkImage; };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc clippy rustfmt pkg-config openssl sqlite e2fsprogs fsverity-utils erofs-utils qemu_kvm python3 jq btrfs-progs ];
      };
    };
}
