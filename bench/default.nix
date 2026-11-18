# Comparison benchmarks (not part of `nix flake check`): the Plasma desktop
# system, composefs store vs. ordinary store, in KVM with host caching off.
#   nix build .#benchmarks.x86_64-linux.desktop
{ pkgs, self, mkImage }:
let
  sys = n: self.nixosConfigurations.${n}.config;
  images = {
    composefs = mkImage { config = sys "bench-plasma"; name = "bench-cfs"; };
    composefs-sha512 = mkImage { config = sys "bench-plasma-sha512"; name = "bench-cfs512"; };
    plain = mkImage { config = sys "bench-plasma-plain"; name = "bench-plain"; };
  };
  qemu = "${pkgs.qemu_test}/bin/qemu-system-x86_64";
  runs = [
    { variant = "composefs"; cpu = "host"; }
    { variant = "plain"; cpu = "host"; }
    { variant = "composefs-sha512"; cpu = "host"; }
    # Skylake without SHA-NI (like the Surface Pro 4's i5-6300U/i7-6650U).
    { variant = "composefs"; cpu = "Skylake-Client-v4"; }
    { variant = "composefs-sha512"; cpu = "Skylake-Client-v4"; }
    { variant = "plain"; cpu = "Skylake-Client-v4"; }
  ];
in
{
  desktop = pkgs.testers.runNixOSTest {
    name = "nixcfs-bench-desktop";
    nodes = { };
    globalTimeout = 4 * 3600;
    testScript = ''
      import os, subprocess
      out = os.environ["out"]
      tmp = os.environ.get("TMPDIR", "/tmp")
      images = { ${pkgs.lib.concatStringsSep ", " (pkgs.lib.mapAttrsToList (k: v: ''"${k}": "${v}/disk.raw"'') images)} }
      runs = [ ${pkgs.lib.concatMapStringsSep ", " (r: ''("${r.variant}", "${r.cpu}")'') runs} ]
      for (variant, cpu) in runs:
          tag = f"{variant}-{cpu}"
          disk = os.path.join(tmp, f"{tag}.qcow2")
          vars = os.path.join(tmp, f"{tag}-vars.fd")
          subprocess.run(["${pkgs.qemu-utils}/bin/qemu-img", "create", "-q", "-f", "qcow2", "-F", "raw", "-b", images[variant], disk, "40G"], check=True)
          subprocess.run(["cp", "${pkgs.OVMF.variables}", vars], check=True)
          os.chmod(vars, 0o644)
          cmd = ("${qemu} -enable-kvm -cpu " + cpu + " -m 6144 -smp 4 "
                 "-drive if=pflash,format=raw,unit=0,readonly=on,file=${pkgs.OVMF.firmware} "
                 f"-drive if=pflash,format=raw,unit=1,file={vars} "
                 f"-drive file={disk},if=virtio,format=qcow2,cache=none,aio=native ")
          m = create_machine(cmd, name=tag.replace(".", "_"))
          # First boot does one-time work (growpart, legacy-store cleanup):
          # measure the second boot.
          for boot in (1, 2):
              m.start()
              m.wait_for_unit("graphical.target", timeout=900)
              m.wait_until_succeeds("systemctl is-system-running --wait | grep -qE 'running|degraded'", timeout=600)
              if boot == 1:
                  m.succeed("sleep 20; sync")
                  m.shutdown()
          m.succeed("sleep 30")
          m.succeed("nixcfs-bench /root/report >/dev/null 2>&1")
          d = os.path.join(out, tag)
          os.makedirs(d, exist_ok=True)
          for f in ["summary.txt", "timings.txt", "boot-analyze.txt", "boot-blame.txt", "boot-critical-chain.txt", "disk.txt", "system.txt", "tamper.txt", "nixcfs-store.log", "boot-report.json"]:
              rc, txt = m.execute(f"cat /root/report/{f}")
              if rc == 0:
                  open(os.path.join(d, f), "w").write(txt)
          print(open(os.path.join(d, "summary.txt")).read())
          m.shutdown()
    '';
  };
}
