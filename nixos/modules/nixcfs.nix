# Serve /nix/store through composefs: an EROFS metadata image over a
# content-addressed, fs-verity protected object store, mounted with overlayfs
# (verity=require) plus a writable upper layer for paths Nix adds at runtime.
#
# Trust anchor: `nixcfs.image=<digest>` on the kernel command line, written
# into every boot entry by the systemd-boot install hook after sealing.
{ config, lib, pkgs, ... }:
let
  cfg = config.nixcfs;
  nixcfs = lib.getExe cfg.package;
  esp = config.boot.loader.efi.efiSysMountPoint;
in
{
  options.nixcfs = {
    enable = lib.mkEnableOption "a composefs-backed /nix/store with fs-verity enforcement";
    package = lib.mkOption {
      type = lib.types.package;
      description = "The nixcfs package.";
    };
    algorithm = lib.mkOption {
      type = lib.types.enum [ "fsverity-sha256-12" "fsverity-sha512-12" ];
      default = "fsverity-sha256-12";
      description = ''
        fs-verity digest algorithm, used when the object store is created (the
        first bootloader install with nixcfs enabled).  Fixed afterwards.
      '';
    };
    narCheck = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        When sealing, check every newly sealed store path against the NAR
        hash recorded in the Nix database (reads the new content once more).
      '';
    };
    sealAfterGC = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Re-seal and update boot entries after the nix-gc service runs, so that
        deleted store paths leave the image and their objects can be freed.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = config.boot.initrd.systemd.enable;
        message = "nixcfs requires boot.initrd.systemd.enable = true";
      }
      {
        assertion = config.boot.loader.systemd-boot.enable;
        message = "nixcfs currently supports systemd-boot only (it stamps the image digest into BLS entries)";
      }
      {
        assertion = !(config.nix.settings.auto-optimise-store or false);
        message = ''
          nixcfs: nix.settings.auto-optimise-store must be off. Hard-linking files
          that live in the composefs lower layer copies them up into the writable
          layer; composefs already deduplicates file contents by digest.
        '';
      }
    ];

    environment.systemPackages = [ cfg.package ];

    # Marks systems whose initrd can mount the composefs store; boot entries
    # for systems without it (pre-conversion generations) are removed.
    system.systemBuilderCommands = ''
      echo ${cfg.package} > $out/nixcfs
    '';

    boot.initrd.kernelModules = [ "erofs" "overlay" ];
    boot.initrd.systemd.storePaths = [ nixcfs ];

    # Mount /sysroot/nix/store from the image named on the command line.
    # There is deliberately no fallback: if verification fails, boot stops.
    boot.initrd.systemd.services.nixcfs-store = {
      description = "Mount the composefs Nix store (fs-verity enforced)";
      unitConfig = {
        DefaultDependencies = false;
        RequiresMountsFor = [ "/sysroot/nix/.cfs" "/sysroot/nix/.cfs-rw" ];
        OnFailure = [ "emergency.target" ];
        OnFailureJobMode = "isolate";
      };
      requiredBy = [ "initrd-fs.target" "initrd-find-nixos-closure.service" ];
      before = [ "initrd-fs.target" "initrd-find-nixos-closure.service" ];
      after = [ "sysroot.mount" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${nixcfs} --root /sysroot boot";
        StandardOutput = "journal+console";
        StandardError = "journal+console";
      };
    };

    # Seal after systemd-boot has written its entries, then stamp the digest
    # on every entry and drop unreferenced images/objects.  If sealing fails,
    # the install fails (nixos-rebuild reports an error) and entries that
    # would boot an unsealed system are removed by the next successful run.
    boot.loader.systemd-boot.extraInstallCommands = ''
      ${nixcfs} update-boot --esp ${esp} --init-algorithm ${cfg.algorithm} ${lib.optionalString (!cfg.narCheck) "--no-nar-check"} >/dev/null
    '';

    # Nix must not hard-link into .links (see assertion) and must leave the
    # store root alone; everything else about Nix is unchanged.
    nix.settings.auto-optimise-store = false;

    systemd.services.nixcfs-cleanup = {
      description = "Remove store content scrubbed from the composefs upper layer";
      wantedBy = [ "multi-user.target" ];
      after = [ "local-fs.target" ];
      serviceConfig = {
        Type = "oneshot";
        IOSchedulingClass = "idle";
        CPUSchedulingPolicy = "idle";
        ExecStart = pkgs.writeShellScript "nixcfs-cleanup" ''
          set -eu
          for d in /nix/.cfs-rw/trash/*; do
            [ -e "$d" ] || continue
            ${pkgs.coreutils}/bin/chmod -R u+w "$d" 2>/dev/null || true
            ${pkgs.coreutils}/bin/rm -rf --one-file-system "$d"
          done
        '';
      };
    };

    # Objects only referenced by the previously booted image are kept while it
    # is mounted; after a reboot they can go.
    systemd.services.nixcfs-gc = {
      description = "Garbage-collect unreferenced composefs images and objects";
      wantedBy = [ "multi-user.target" ];
      after = [ "local-fs.target" "boot.mount" ];
      serviceConfig = {
        Type = "oneshot";
        IOSchedulingClass = "idle";
        ExecStart = "${nixcfs} gc --esp ${esp}";
      };
    };

    systemd.services.nix-gc = lib.mkIf (config.nix.gc.automatic && cfg.sealAfterGC) {
      serviceConfig.ExecStartPost = [ "${nixcfs} update-boot --esp ${esp}" ];
    };
  };
}
