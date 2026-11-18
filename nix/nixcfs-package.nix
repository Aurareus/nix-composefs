# nixcfs, built against composefs-rs (flake input) with our patches applied.
{ lib, rustPlatform, applyPatches, runCommand, pkg-config, openssl, composefs-rs-src }:
let
  composefs-rs = applyPatches {
    name = "composefs-rs-patched";
    src = composefs-rs-src;
    patches = lib.filesystem.listFilesRecursive ../patches/composefs-rs;
  };
  src = runCommand "nixcfs-src" { } ''
    mkdir -p $out/third_party
    cp -r ${lib.fileset.toSource { root = ../nixcfs; fileset = lib.fileset.unions [ ../nixcfs/Cargo.toml ../nixcfs/Cargo.lock ../nixcfs/src ]; }} $out/nixcfs
    ln -s ${composefs-rs} $out/third_party/composefs-rs
  '';
in
rustPlatform.buildRustPackage {
  pname = "nixcfs";
  version = "0.1.0";
  inherit src;
  sourceRoot = "nixcfs-src/nixcfs";
  cargoLock.lockFile = ../nixcfs/Cargo.lock;
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ openssl ];
  doCheck = false;
  passthru = { inherit composefs-rs; };
  meta = {
    description = "Serve the Nix store through composefs";
    mainProgram = "nixcfs";
    license = with lib.licenses; [ mit asl20 ];
  };
}
