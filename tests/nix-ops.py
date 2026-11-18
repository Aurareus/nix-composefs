# Nix keeps working on the running system: build, add paths, GC; and GC
# actually frees the object store.
machine.start()
machine.wait_for_unit("multi-user.target")
d1 = check_verified_store()

with subtest("sandboxed build"):
    expr = ('derivation { name = "nixcfs-test-build"; system = "x86_64-linux"; builder = "/bin/sh"; '
            'args = [ "-c" "echo built-inside-the-vm > $out" ]; }')
    out = machine.succeed(f"nix-build --option sandbox true -E '{expr}' --no-out-link").strip()
    assert machine.succeed(f"cat {out}").strip() == "built-inside-the-vm"
    built = out

with subtest("nix-store --add and nix store add-file"):
    machine.succeed("head -c 1000000 /dev/urandom > /root/blob")
    added = machine.succeed("nix-store --add /root/blob").strip()
    machine.succeed(f"cmp /root/blob {added}")
    added2 = machine.succeed("echo hi > /root/hi && nix store add-file /root/hi").strip()
    machine.succeed(f"grep -q hi {added2}")

with subtest("sealing picks up runtime-added paths"):
    machine.succeed(f"nix-store --add-root /root/keep-blob --indirect -r {added}")
    rep = json.loads(machine.succeed("nixcfs update-boot --esp /boot"))
    print(rep["seal"])
    assert rep["seal"]["scanned_paths"] >= 3, rep["seal"]
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    d2 = check_verified_store()
    assert d2 == rep["seal"]["image"]
    machine.succeed(f"cmp /root/blob {added}")
    name = added.split("/")[3]
    machine.succeed(f"! test -e /nix/.cfs-rw/upper/{name}")

with subtest("garbage collection removes dead paths and frees their objects"):
    machine.succeed("mkdir -p /mnt/gen2cache && mount -t 9p -o trans=virtio,version=9p2000.L,ro gen2cache /mnt/gen2cache")
    machine.succeed(f"nix-store -r {GEN2} --option substituters file:///mnt/gen2cache --option require-sigs false")
    machine.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {GEN2} && {GEN2}/bin/switch-to-configuration boot")
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    cowsay_path = machine.succeed(f"nix-store -qR {GEN1} | grep -- '-cowsay-' | head -1").strip()
    assert cowsay_path
    # Objects that only cowsay uses (perl script etc.).
    cowsay_bin = cowsay_path + "/bin/cowsay"
    obj = machine.succeed(f"nixcfs object-of {cowsay_bin}").strip()
    machine.succeed(f"test -f {obj}")
    used_before = int(machine.succeed("du -sb /nix/.cfs/objects | cut -f1"))
    machine.succeed("nix-env -p /nix/var/nix/profiles/system --delete-generations old")
    machine.succeed("rm -f /root/keep-blob")
    # The NixOS GC service; nixcfs hooks it to re-seal and update boot entries.
    machine.succeed("systemctl start nix-gc.service")
    print(machine.succeed("journalctl -u nix-gc.service | tail -5"))
    machine.succeed("journalctl -u nix-gc.service | grep -q 'nixcfs: sealed'")
    machine.succeed(f"! test -e {cowsay_path}")
    machine.succeed(f"! test -e {added}")
    machine.succeed(f"! test -e {built}")
    rep = json.loads(machine.succeed("nixcfs status"))
    d3 = rep["latest_image"]
    assert d3 != d2
    # The booted image still holds the objects while mounted...
    machine.succeed(f"test -f {obj}")
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    assert check_verified_store() == d3
    machine.wait_until_succeeds("systemctl show -p ActiveState --value nixcfs-gc.service | grep -qx inactive")
    print(machine.succeed("journalctl -u nixcfs-gc.service | tail -5"))
    # ...and after rebooting into the new image they are gone.
    machine.succeed(f"! test -e {obj}")
    used_after = int(machine.succeed("du -sb /nix/.cfs/objects | cut -f1"))
    print(f"objects: {used_before} -> {used_after} bytes")
    assert used_after < used_before - 1000000
    assert machine.succeed("ls /nix/.cfs/images | wc -l").strip() in ("1", "2")
    machine.succeed("nix-store --verify --check-contents")
machine.shutdown()
