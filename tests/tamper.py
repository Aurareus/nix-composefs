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
        off = int(re.search(r"physical (\d+)", out).group(1)) + 8192
    machine.succeed(f"printf 'TAMPERED' | dd of=/dev/vda2 bs=1 seek={off} conv=notrunc,fsync status=none")
    must_refuse(big, "raw block write")
    print(machine.succeed("dmesg | grep -iE 'verity|csum' | tail -3"))

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
    machine.wait_for_console_text("nixcfs-store.service: Failed|Failed to start.*composefs Nix store|Verifying fs-verity digest", timeout=300)
    machine.wait_for_console_text("emergency|Kernel panic", timeout=120)
    print("boot refused as expected")
    machine.crash()
