machine.start()
machine.wait_for_unit("multi-user.target")
d = check_verified_store()
print("booted image", d)
machine.succeed("nix-store --verify --check-contents")
machine.succeed('test "$(cat /etc/nixcfs-generation)" = 1')
machine.succeed("cowsay moo")
# Nothing of the booted system lives in the writable layer.
st = json.loads(machine.succeed("nixcfs status"))
print(st)
assert st["booted_image"] == d and st["latest_image"] == d, st
assert st["shadowing_sealed_paths"] == [], st
machine.succeed("systemctl show -p Result --value nixcfs-cleanup.service | grep -qx success")
machine.succeed("systemctl show -p Result --value nixcfs-gc.service | grep -qx success")
machine.succeed("! test -e /nix/.cfs-rw/trash/* ")
print(machine.succeed("cat /run/nixcfs/boot-report.json"))
print(machine.succeed("systemd-analyze"))
machine.shutdown()
