# Failure handling: a seal that fails (here: a store path whose content no
# longer matches its NAR hash) must not leave the machine unbootable, and a
# power cut right after a rebuild must not either.
machine.start()
machine.wait_for_unit("multi-user.target")
d1 = check_verified_store()

with subtest("corrupted unsealed path: seal refuses, entries stay bootable"):
    p = machine.succeed("head -c 200000 /dev/urandom > /root/f && nix-store --add /root/f").strip()
    name = p.split("/")[3]
    machine.succeed(f"nix-store --add-root /root/keep --indirect -r {p}")
    # Corrupt it behind Nix's back (in the writable layer, before sealing).
    machine.succeed(f"chmod u+w /nix/.cfs-rw/upper/{name} && printf X | dd of=/nix/.cfs-rw/upper/{name} bs=1 seek=10 conv=notrunc status=none")
    rc, out = machine.execute("/run/current-system/bin/switch-to-configuration boot 2>&1")
    print(out)
    assert rc != 0, "switch-to-configuration should fail when sealing fails"
    assert "NAR hash mismatch" in out or "SEALING FAILED" in out, out
    entries = machine.succeed("grep -h '^options' /boot/loader/entries/nixos-generation-*.conf")
    assert f"nixcfs.image={d1}" in entries, entries
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    assert check_verified_store() == d1
    # Recover: drop the broken path; sealing works again.
    machine.succeed(f"rm /root/keep && nix-store --delete {p}")
    machine.succeed("/run/current-system/bin/switch-to-configuration boot")

with subtest("power loss right after a rebuild"):
    machine.succeed("mkdir -p /mnt/gen2cache && mount -t 9p -o trans=virtio,version=9p2000.L,ro gen2cache /mnt/gen2cache")
    machine.succeed(f"nix-store -r {GEN2} --option substituters file:///mnt/gen2cache --option require-sigs false")
    machine.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {GEN2} && {GEN2}/bin/switch-to-configuration boot")
    machine.crash()      # no clean shutdown, no sync beyond what the tools did
    machine.start()
    machine.wait_for_unit("multi-user.target")
    check_verified_store()
    assert machine.succeed("cat /etc/nixcfs-generation").strip() == "2"
    machine.succeed("nix-store --verify --check-contents")
machine.shutdown()
