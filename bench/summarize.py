#!/usr/bin/env python3
"""Summarise nixcfs-bench report directories into Markdown tables.

usage: summarize.py DIR...   (each DIR holds timings.txt, boot-analyze.txt, ...)
"""
import json, os, re, statistics, sys


def timings(d):
    t = {}
    for line in open(os.path.join(d, "timings.txt")):
        name, secs, _ = line.split()
        base = re.sub(r"_\d+$", "", name)
        t.setdefault(base, []).append(float(secs))
    return {k: statistics.median(v) for k, v in t.items()}


def boot(d):
    s = open(os.path.join(d, "boot-analyze.txt")).read()
    def g(rx):
        m = re.search(rx, s)
        return float(m.group(1)) if m else None
    def secs(x):
        # "1min 2.345s" or "2.345s" or "345ms"
        m = re.search(r"((\d+)min )?([\d.]+)(ms|s)", x)
        v = float(m.group(3)) / (1000 if m.group(4) == "ms" else 1)
        return v + 60 * int(m.group(2) or 0)
    out = {}
    m = re.search(r"in (.*?) \(kernel\) \+ (.*?) \(initrd\) \+ (.*?) \(userspace\) = (.*?)\s*$", s, re.M)
    if m:
        out = dict(kernel=secs(m.group(1)), initrd=secs(m.group(2)), userspace=secs(m.group(3)), total=secs(m.group(4)))
    m = re.search(r"graphical.target reached after (.*?) in userspace", s)
    if m:
        out["graphical"] = secs(m.group(1))
    return out


def disk(d):
    s = open(os.path.join(d, "disk.txt")).read()
    out = {}
    m = re.search(r"Used:\s+(\d+)", s)   # btrfs filesystem usage -b
    if m:
        out["fs_used"] = int(m.group(1))
    for k, rx in [("objects_MiB", r"(\d+) /nix/.cfs/objects"), ("images_MiB", r"(\d+) /nix/.cfs/images"),
                  ("store_MiB", r"(\d+) /nix/store\n"), ("objects", r"objects: (\d+)")]:
        m = re.search(rx, s)
        if m:
            out[k] = int(m.group(1))
    sizes = re.findall(r"^(\d+) /nix/.cfs/images/[0-9a-f]+$", s, re.M)
    if sizes:
        out["image_bytes"] = max(int(x) for x in sizes)
    return out


def rebuild(d):
    p = os.path.join(d, "rebuild.txt")
    if not os.path.exists(p):
        return {}
    out = {}
    for line in open(p):
        k, v = line.split()
        out[k] = float(v)
    j = os.path.join(d, "last-update-boot.json")
    if os.path.exists(j) and os.path.getsize(j) > 0:
        try:
            r = json.load(open(j))
            out["seal_s"] = r["seal"]["seconds"]
            out["sealed_new_paths"] = r["seal"]["scanned_paths"]
            out["sealed_new_bytes"] = r["seal"]["import"]["bytes"]
        except Exception:
            pass
    return out


def main(dirs):
    rows = {os.path.basename(d.rstrip("/")): (timings(d), boot(d), disk(d), rebuild(d)) for d in dirs}
    names = list(rows)
    print("| metric | " + " | ".join(names) + " |")
    print("|---|" + "---|" * len(names))
    keys = []
    for t, *_ in rows.values():
        for k in t:
            if k not in keys:
                keys.append(k)
    def line(label, f):
        vals = []
        for n in names:
            try:
                v = f(rows[n])
                vals.append("" if v is None else (f"{v:.3f}" if isinstance(v, float) else str(v)))
            except Exception:
                vals.append("")
        print(f"| {label} | " + " | ".join(vals) + " |")
    for k in ["kernel", "initrd", "userspace", "total", "graphical"]:
        line(f"boot {k} (s)", lambda r, k=k: r[1].get(k))
    for k in keys:
        line(f"{k} (s, median of 3)", lambda r, k=k: r[0].get(k))
    for k in ["fs_used", "store_MiB", "objects_MiB", "images_MiB", "image_bytes", "objects"]:
        line(k, lambda r, k=k: r[2].get(k))
    for k in ["fetch_s", "switch_to_configuration_boot_s", "noop_switch_to_configuration_boot_s", "seal_s", "sealed_new_paths", "sealed_new_bytes"]:
        line(k, lambda r, k=k: r[3].get(k))


if __name__ == "__main__":
    main(sys.argv[1:])
