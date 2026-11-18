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
  gen2 = {
    composefs = (sys "bench-plasma-gen2").system.build.toplevel;
    composefs-sha512 = (sys "bench-plasma-sha512-gen2").system.build.toplevel;
    plain = (sys "bench-plasma-plain-gen2").system.build.toplevel;
  };
  gen2Cache = pkgs.mkBinaryCache { compression = "none"; rootPaths = builtins.attrValues gen2; };
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
      gen2 = { ${pkgs.lib.concatStringsSep ", " (pkgs.lib.mapAttrsToList (k: v: ''"${k}": "${v}"'') gen2)} }
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
                 f"-drive file={disk},if=virtio,format=qcow2,cache=none,aio=native "
                 "-virtfs local,path=${gen2Cache},security_model=none,readonly=on,mount_tag=gen2cache ")
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
          # Rebuild cost: an update adding a few GB of new paths, then a no-op.
          g2 = gen2[variant]
          m.succeed("mkdir -p /mnt/c && mount -t 9p -o trans=virtio,version=9p2000.L,ro,msize=1048576 gen2cache /mnt/c")
          fetch = m.succeed(f"/run/current-system/sw/bin/time -f %e nix-store -r {g2} --option substituters file:///mnt/c --option require-sigs false 2>&1 >/dev/null | tail -1").strip()
          m.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {g2}")
          sw = m.succeed(f"sync; echo 3 > /proc/sys/vm/drop_caches; /run/current-system/sw/bin/time -f %e {g2}/bin/switch-to-configuration boot 2>&1 >/dev/null | tail -1").strip()
          rep = m.execute("cat /nix/.cfs-rw/last-update-boot.json")[1]
          noop = m.succeed(f"/run/current-system/sw/bin/time -f %e {g2}/bin/switch-to-configuration boot 2>&1 >/dev/null | tail -1").strip()
          new = m.succeed(f"nix-store -qR {g2} | wc -l").strip()
          with open(os.path.join(d, "rebuild.txt"), "w") as f:
              f.write(f"fetch_s {fetch}\nswitch_to_configuration_boot_s {sw}\nnoop_switch_to_configuration_boot_s {noop}\ngen2_closure_paths {new}\n")
          open(os.path.join(d, "last-update-boot.json"), "w").write(rep)
          print(open(os.path.join(d, "rebuild.txt")).read())
          m.shutdown()
    '';
  };
}
