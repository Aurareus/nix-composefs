# Converting an existing NixOS install with an ordinary /nix/store: enable the
# module, install the bootloader, reboot.  This is how a running machine
# (e.g. a fresh NixOS install on btrfs) moves to the composefs store.
machine.start()
machine.wait_for_unit("multi-user.target")
assert "overlay" not in machine.succeed("findmnt -n -o FSTYPE /nix/store || echo plain")
plain_sys = machine.succeed("readlink -f /run/current-system").strip()
machine.succeed("cowsay still plain")

with subtest("enable nixcfs and install the bootloader"):
    machine.succeed("mkdir -p /mnt/gen2cache && mount -t 9p -o trans=virtio,version=9p2000.L,ro gen2cache /mnt/gen2cache")
    machine.succeed(f"nix-store -r {GEN1} --option substituters file:///mnt/gen2cache --option require-sigs false")
    out = machine.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {GEN1} && {GEN1}/bin/switch-to-configuration boot 2>&1")
    print(out)
    assert "creating repository" in out, out
    machine.succeed("grep -q nixcfs.image= /boot/loader/entries/nixos-generation-2.conf")
    # The pre-conversion generation cannot mount the composefs store (its
    # initrd predates it), so its entry is removed rather than left broken.
    machine.succeed("! test -e /boot/loader/entries/nixos-generation-1.conf")
    assert "predates nixcfs" in out, out

with subtest("reboot: store is now composefs, old store migrated away"):
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    check_verified_store()
    rep = json.loads(machine.succeed("cat /run/nixcfs/boot-report.json"))
    assert rep["scrub"]["legacy_trashed"], rep
    machine.wait_until_succeeds("systemctl show -p ActiveState --value nixcfs-cleanup.service | grep -qx inactive", timeout=600)
    machine.succeed("test -z \"$(ls -A /nix/.cfs-rw/trash)\"")
    machine.succeed("nix-store --verify --check-contents")
    machine.succeed("cowsay converted")

with subtest("old generations built with nixcfs keep booting after conversion"):
    machine.succeed(f"nix-store -r {GEN2} --option substituters file:///mnt/gen2cache --option require-sigs false || "
                    "(mkdir -p /mnt/gen2cache && mount -t 9p -o trans=virtio,version=9p2000.L,ro gen2cache /mnt/gen2cache && "
                    f"nix-store -r {GEN2} --option substituters file:///mnt/gen2cache --option require-sigs false)")
    machine.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {GEN2} && {GEN2}/bin/switch-to-configuration boot")
    machine.succeed("bootctl set-oneshot nixos-generation-2.conf")
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    check_verified_store()
    assert machine.succeed("readlink -f /run/booted-system").strip() == GEN1
    assert plain_sys != GEN1
machine.shutdown()
