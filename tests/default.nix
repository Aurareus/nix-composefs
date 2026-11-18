# Automated VM tests.  Each test boots real disk images (UEFI, systemd-boot,
# btrfs root by default) built by nixos/lib/disk-image.nix.
{ pkgs, self, mkImage }:
let
  lib = pkgs.lib;
  gen1 = self.nixosConfigurations.vm-gen1.config;
  gen2 = self.nixosConfigurations.vm-gen2.config;
  qemu = "${pkgs.qemu_test}/bin/qemu-system-x86_64";

  mkTest = { name, fsType ? "btrfs", script, extra ? { } }:
    let
      image = mkImage { config = gen1; inherit fsType; };
      gen2Cache = pkgs.mkBinaryCache { rootPaths = [ gen2.system.build.toplevel ]; };
    in
    pkgs.testers.runNixOSTest ({
      inherit name;
      nodes = { };
      testScript = ''
        import os, subprocess, json, re  # noqa: F401
        tmp = os.environ.get("TMPDIR", "/tmp")
        disk = os.path.join(tmp, "disk.qcow2")
        vars = os.path.join(tmp, "OVMF_VARS.fd")
        subprocess.run(["${pkgs.qemu-utils}/bin/qemu-img", "create", "-q", "-f", "qcow2", "-F", "raw",
                        "-b", "${image}/disk.raw", disk], check=True)
        subprocess.run(["cp", "${pkgs.OVMF.variables}", vars], check=True)
        os.chmod(vars, 0o644)
        GEN1 = "${gen1.system.build.toplevel}"
        GEN2 = "${gen2.system.build.toplevel}"
        GEN2_CACHE = "${gen2Cache}"

        def start_command(extra=""):
            return (
                "${qemu} -enable-kvm -cpu host -m 3072 -smp 2 "
                "-drive if=pflash,format=raw,unit=0,readonly=on,file=${pkgs.OVMF.firmware} "
                f"-drive if=pflash,format=raw,unit=1,file={vars} "
                f"-drive file={disk},if=virtio,format=qcow2 "
                f"-virtfs local,path={GEN2_CACHE},security_model=none,readonly=on,mount_tag=gen2cache "
                + extra
            )

        machine = create_machine(start_command(), name="machine")

        def cmdline_digest():
            m = re.search(r"nixcfs\.image=([0-9a-f]+)", machine.succeed("cat /proc/cmdline"))
            assert m, "no nixcfs.image= on the kernel command line"
            return m.group(1)

        def check_verified_store():
            """The running /nix/store is our overlay with verity=require, from
            the image named on the kernel command line."""
            d = cmdline_digest()
            mnt = machine.succeed("findmnt -n -o FSTYPE,SOURCE,OPTIONS --target /nix/store --first-only --direction backward")
            print(mnt)
            assert "verity=require" in machine.succeed("grep ' /nix/store ' /proc/self/mountinfo | grep overlay | head -1"), mnt
            assert f"composefs:{d}" in machine.succeed("grep ' /nix/store ' /proc/self/mountinfo"), mnt
            assert machine.succeed("cat /run/nixcfs/image").strip() == d
            return d

        FSTYPE = "${fsType}"

        ${script}
      '';
    } // extra);
in
{
  boot = mkTest {
    name = "nixcfs-boot";
    script = builtins.readFile ./boot.py;
  };
  tamper = mkTest {
    name = "nixcfs-tamper";
    script = builtins.readFile ./tamper.py;
  };
  tamper-ext4 = mkTest {
    name = "nixcfs-tamper-ext4";
    fsType = "ext4";
    script = builtins.readFile ./tamper.py;
  };
  lifecycle = mkTest {
    name = "nixcfs-lifecycle";
    script = builtins.readFile ./lifecycle.py;
  };
  nix-ops = mkTest {
    name = "nixcfs-nix-ops";
    script = builtins.readFile ./nix-ops.py;
  };
}
