# Rebuild to a new generation, boot it, roll back (bootloader and runtime).
machine.start()
machine.wait_for_unit("multi-user.target")
d1 = check_verified_store()

def gen():
    return machine.succeed("cat /etc/nixcfs-generation").strip()

def in_upper(store_path):
    name = store_path.split("/")[3]
    return machine.execute(f"test -e /nix/.cfs-rw/upper/{name}")[0] == 0

with subtest("rebuild to generation 2 (switch)"):
    # Stand-in for nixos-rebuild's build step: fetch the prebuilt toplevel
    # from a local binary cache (the VM has no network).
    machine.succeed("mkdir -p /mnt/gen2cache && mount -t 9p -o trans=virtio,version=9p2000.L,ro gen2cache /mnt/gen2cache")
    machine.succeed(f"nix-store -r {GEN2} --option substituters file:///mnt/gen2cache --option require-sigs false")
    assert in_upper(GEN2), "new generation should start out in the writable layer"
    out = machine.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {GEN2} && "
                          f"{GEN2}/bin/switch-to-configuration switch 2>&1")
    print(out)
    assert gen() == "2"
    machine.succeed("hello")
    st = json.loads(machine.succeed("nixcfs status"))
    print(st)
    assert st["booted_image"] == d1 and st["latest_image"] != d1, st
    entries = machine.succeed("grep -h '^options' /boot/loader/entries/nixos-generation-*.conf")
    assert entries.count(f"nixcfs.image={st['latest_image']}") == 2, entries
    d2 = st["latest_image"]

with subtest("boot into generation 2: now verified"):
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    assert check_verified_store() == d2
    assert gen() == "2"
    assert not in_upper(GEN2), "sealed generation still in the writable layer"
    machine.succeed("hello")
    machine.succeed("nix-store --verify --check-contents")

with subtest("roll back through the boot menu to generation 1"):
    machine.succeed("bootctl set-oneshot nixos-generation-1.conf")
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    assert check_verified_store() == d2   # one image serves every entry
    assert gen() == "1"
    machine.succeed("cowsay rolled back")
    assert machine.succeed("readlink /run/booted-system").strip() == GEN1

with subtest("boot generation 2 again, then nixos-rebuild switch --rollback"):
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    assert gen() == "2"
    print(machine.succeed("nixos-rebuild switch --rollback 2>&1"))
    assert gen() == "1"
    machine.succeed("cowsay runtime rollback")
    # Default boot entry follows the profile: next boot is generation 1.
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    check_verified_store()
    assert gen() == "1"
machine.shutdown()
