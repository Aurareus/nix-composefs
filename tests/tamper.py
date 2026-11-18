# Tamper detection: every way of changing sealed store content we can think of
# must result in the content being refused (EIO / failed boot), never served.
machine.start()
machine.wait_for_unit("multi-user.target")
d = check_verified_store()
cowsay = machine.succeed("readlink -f $(command -v cowsay)").strip()
cowsay_path = "/".join(cowsay.split("/")[:4])
cowsay_name = cowsay_path.split("/")[-1]
print("victim:", cowsay)
good = machine.succeed(f"sha256sum {cowsay}").split()[0]

def drop_caches():
    machine.succeed("sync; echo 3 > /proc/sys/vm/drop_caches")

def must_refuse(f, why):
    drop_caches()
    rc, out = machine.execute(f"cat {f} > /dev/null")
    assert rc != 0, f"{why}: tampered content was served"
    print(f"refused ({why}):", machine.succeed("dmesg | tail -3"))

with subtest("object replaced by a modified copy with its own fs-verity"):
    obj = machine.succeed(f"nixcfs object-of {cowsay}").strip()
    machine.succeed(f"cp {obj} /root/obj.orig")
    machine.succeed(f"cp {obj} /root/obj.evil && echo 'print \"pwned\\n\";' >> /root/obj.evil")
    machine.succeed(f"rm -f {obj} && cp /root/obj.evil {obj} && fsverity enable {obj}")
    must_refuse(cowsay, "object swapped, verity on")
    machine.succeed("dmesg | grep -q 'has the wrong fs-verity digest'")

with subtest("object replaced by a modified copy without fs-verity"):
    machine.succeed(f"rm -f {obj} && cp /root/obj.evil {obj}")
    must_refuse(cowsay, "object swapped, verity off")
    machine.succeed("dmesg | grep -q 'has no fs-verity digest'")
    machine.succeed(f"rm -f {obj} && cp /root/obj.orig {obj} && fsverity enable {obj}")
    drop_caches()
    assert machine.succeed(f"sha256sum {cowsay}").split()[0] == good

with subtest("raw block device modification of an object"):
    # A largish file nothing on the system is using (perl's shared library,
    # only loaded when cowsay runs).
    big = machine.succeed(f"find $(nix-store -qR {cowsay_path}) -type f -name 'libperl.so*' -size +64k | head -1").strip()
    assert big, "no victim file"
    bigobj = machine.succeed(f"nixcfs object-of {big}").strip()
    if FSTYPE == "ext4":
        blk = machine.succeed(f"filefrag -v {bigobj} | awk '$1==\"0:\"{{print $4}}' | tr -d .").strip()
        off = int(blk) * 4096 + 8192
    else:
        logical = machine.succeed(f"filefrag -v {bigobj} | awk '$1==\"0:\"{{print $4}}' | tr -d .").strip()
        out = machine.succeed(f"btrfs-map-logical -l {int(logical)*4096} /dev/vda2")
        print(out)
        m = re.search(r"physical (\d+)", out)
        assert m, out
        off = int(m.group(1)) + 8192
    machine.succeed(f"dd if=/dev/vda2 of=/root/saved-bytes bs=1 skip={off} count=8 status=none")
    machine.succeed(f"printf 'TAMPERED' | dd of=/dev/vda2 bs=1 seek={off} conv=notrunc,fsync status=none")
    must_refuse(big, "raw block write")
    print(machine.succeed("dmesg | grep -iE 'verity|csum' | tail -3"))
    if FSTYPE == "ext4":
        machine.succeed("dmesg | grep -q 'FILE CORRUPTED'")   # caught by fs-verity itself
    # Repair the block so the rest of the test (and perl) keeps working.
    machine.succeed(f"dd if=/root/saved-bytes of=/dev/vda2 bs=1 seek={off} conv=notrunc,fsync status=none")
    drop_caches()
    machine.succeed(f"cat {big} > /dev/null")

with subtest("writable layer cannot shadow sealed paths (offline tamper)"):
    # Simulate an attacker editing the disk while the system is down: plant a
    # replacement for a sealed path in the overlay upper directory.
    machine.succeed(f"mkdir -p /nix/.cfs-rw/upper/{cowsay_name}/bin")
    machine.succeed(f"printf '#!/bin/sh\\necho pwned\\n' > /nix/.cfs-rw/upper/{cowsay_name}/bin/cowsay")
    machine.succeed(f"chmod +x /nix/.cfs-rw/upper/{cowsay_name}/bin/cowsay; sync")
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    check_verified_store()
    rep = json.loads(machine.succeed("cat /run/nixcfs/boot-report.json"))
    print(rep)
    assert cowsay_name in rep["scrub"]["removed_shadowing"], rep
    assert "pwned" not in machine.succeed("cowsay hi")
    assert machine.succeed(f"sha256sum {cowsay}").split()[0] == good

with subtest("runtime modification by root is reported and reverted at boot"):
    machine.succeed("mount -o remount,rw,bind /nix/store")
    machine.succeed(f"chmod u+w {cowsay} && echo '# modified' >> {cowsay}")
    assert machine.succeed(f"sha256sum {cowsay}").split()[0] != good
    st = json.loads(machine.succeed("nixcfs status"))
    assert cowsay_name in st["shadowing_sealed_paths"], st
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("multi-user.target")
    assert machine.succeed(f"sha256sum {cowsay}").split()[0] == good
    assert json.loads(machine.succeed("nixcfs status"))["shadowing_sealed_paths"] == []

with subtest("tampered image: boot refuses (hard fail)"):
    img = f"/nix/.cfs/images/{d}"
    real = machine.succeed(f"readlink -f {img}").strip()
    # Replace the image object with a modified copy (verity enabled, so only
    # the digest comparison against the kernel command line can catch it).
    machine.succeed(f"cp {real} /root/img && printf X | dd of=/root/img bs=1 seek=2000 conv=notrunc status=none")
    machine.succeed(f"rm -f {real} && cp /root/img {real} && fsverity enable {real} && sync")
    machine.shutdown()
    machine.start()
    # (Polling the full console log: the driver's wait_for_console_text falls
    # behind fast output; see patches/nixpkgs/0001.)
    def console_has(rx):
        return lambda _: re.search(rx, machine.get_console_log()) is not None
    retry(console_has(r"Expected digest [0-9a-f]+ but found [0-9a-f]+"), 300)
    retry(console_has(r"Failed to start Mount the composefs Nix store"), 60)
    retry(console_has(r"emergency|Kernel panic"), 60)
    # The system never got as far as mounting the store or switching root.
    assert "Reached target Switch Root" not in machine.get_console_log().split("Expected digest")[-1]
    print("boot refused as expected")
    machine.crash()
