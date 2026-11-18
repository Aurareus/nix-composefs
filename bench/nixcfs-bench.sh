#!/usr/bin/env bash
# nixcfs-bench: measure a booted system (composefs store or ordinary store) and
# write a report directory.  Run as root, a few minutes after boot, with nothing
# else running.  Usage: nixcfs-bench [OUTDIR]   (default: /boot/nixcfs-reports/...)
set -uo pipefail
set +o errexit   # (writeShellApplication turns it on; failures are recorded, not fatal)
if [ "$(id -u)" != 0 ]; then exec sudo "$0" "$@"; fi

variant=plain
grep -q ' /nix/store overlay .*verity=require' /proc/self/mountinfo 2>/dev/null && variant=composefs
grep -qE '/nix/store .*(verity=require)' /proc/self/mountinfo && variant=composefs
ts=$(date -u +%Y%m%dT%H%M%SZ)
out=${1:-/boot/nixcfs-reports/$(hostname)-$variant-$ts}
mkdir -p "$out"
exec > >(tee "$out/summary.txt") 2>&1
echo "nixcfs-bench variant=$variant host=$(hostname) at $ts"

dropc() { sync; echo 3 > /proc/sys/vm/drop_caches; }
# t NAME CMD... : run CMD, record wall-clock seconds as NAME
t() {
  local name=$1; shift
  local s e
  local rc=0
  s=$(date +%s.%N); "$@" > /dev/null 2>&1 || rc=$?; e=$(date +%s.%N)
  printf '%s %.3f rc=%d\n' "$name" "$(echo "$e - $s" | bc)" "$rc" | tee -a "$out/timings.txt"
}

{
  uname -a; cat /proc/cmdline; echo; lscpu; echo; free -m; echo
  grep -E ' /nix/store ' /proc/self/mountinfo; echo
  findmnt -o TARGET,SOURCE,FSTYPE,OPTIONS /; lsblk -o NAME,SIZE,MODEL,TRAN,FSTYPE,LABEL
  grep -o 'sha256-[a-z0-9]*\|sha512-[a-z0-9]*' /proc/crypto | sort -u
} > "$out/system.txt" 2>&1

# --- boot ---------------------------------------------------------------
systemd-analyze > "$out/boot-analyze.txt" 2>&1
systemd-analyze blame > "$out/boot-blame.txt" 2>&1
systemd-analyze critical-chain > "$out/boot-critical-chain.txt" 2>&1
journalctl -b -o short-monotonic -u nixcfs-store.service > "$out/nixcfs-store.log" 2>&1
cp /run/nixcfs/boot-report.json "$out/" 2>/dev/null
cat "$out/boot-analyze.txt"

# --- disk use -----------------------------------------------------------
{
  df -B1 /; echo
  if [ "$(findmnt -n -o FSTYPE /)" = btrfs ]; then btrfs filesystem usage -b / ; fi
  echo; echo "== du (apparent, MiB)"
  for d in /nix/store /nix/.cfs/objects /nix/.cfs/images /nix/.cfs-rw/upper; do
    [ -e "$d" ] && printf '%s %s\n' "$(du -sxm --apparent-size "$d" 2>/dev/null | cut -f1)" "$d"
  done
  echo "== du (allocated, MiB, -x)"
  [ -d /nix/.cfs ] && du -sxm /nix/.cfs /nix/.cfs-rw
  [ -d /nix/.cfs ] || du -sxm /nix/store
  echo "store paths: $(ls /nix/store | wc -l)"
  [ -d /nix/.cfs/objects ] && echo "objects: $(find /nix/.cfs/objects -type f | wc -l)"
  [ -d /nix/.cfs/images ] && ls -la /nix/.cfs/images/ && for i in /nix/.cfs/images/[0-9a-f]*; do stat -L -c '%s %n' "$i"; done
} > "$out/disk.txt" 2>&1

# --- file access --------------------------------------------------------
sys=$(readlink -f /run/current-system)
nix-store -qR "$sys" > "$out/closure.txt"
echo "closure: $(wc -l < "$out/closure.txt") paths, $(nix path-info -S "$sys" 2>/dev/null | awk '{print $2}') bytes"
readall() { xargs -a "$out/closure.txt" -I{} find {} -type f -print0 | xargs -0 -P4 cat > /dev/null; }
statall() { xargs -a "$out/closure.txt" -I{} find {} -printf '' ; }
for i in 1 2 3; do dropc; t "read_closure_cold_$i" readall; done
for i in 1 2 3; do t "read_closure_warm_$i" readall; done
for i in 1 2 3; do dropc; t "walk_closure_cold_$i" statall; done
for i in 1 2 3; do t "walk_closure_warm_$i" statall; done
apps=()
[ -x "$sys/sw/bin/python3" ] && apps+=("python3:${sys}/sw/bin/python3 -c import\ json,ssl,sqlite3,asyncio")
[ -x "$sys/sw/bin/perl" ] && apps+=("perl:${sys}/sw/bin/perl -MPOSIX -e1")
[ -x "$sys/sw/bin/konsole" ] && apps+=("konsole:env QT_QPA_PLATFORM=offscreen $sys/sw/bin/konsole --version")
[ -x "$sys/sw/bin/kate" ] && apps+=("kate:env QT_QPA_PLATFORM=offscreen $sys/sw/bin/kate --version")
[ -x "$sys/sw/bin/firefox" ] && apps+=("firefox:$sys/sw/bin/firefox --headless --screenshot /tmp/ff.png about:blank")
for a in "${apps[@]}"; do
  n=${a%%:*}; c=${a#*:}
  for i in 1 2 3; do dropc; t "app_${n}_cold_$i" bash -c "$c"; done
  for i in 1 2 3; do t "app_${n}_warm_$i" bash -c "$c"; done
done
dropc; t verify_check_contents nix-store --verify --check-contents

# --- integrity (composefs only; non-destructive) -------------------------
if [ $variant = composefs ]; then
  {
    nixcfs status
    victim=$(find "$(readlink -f "$sys/sw/bin/perl" | cut -d/ -f1-4)" -type f -size +64k | head -1)
    obj=$(nixcfs object-of "$victim")
    echo "victim $victim -> $obj"
    cp "$obj" /root/obj.orig && cp "$obj" /root/obj.evil && echo pwned >> /root/obj.evil
    rm -f "$obj" && cp /root/obj.evil "$obj" && fsverity enable "$obj"
    dropc
    if cat "$victim" > /dev/null 2>&1; then echo "TAMPER TEST FAILED: modified object served"; else echo "tamper test ok: modified object refused"; fi
    dmesg | grep -i 'fs-verity\|overlayfs' | tail -3
    rm -f "$obj" && cp /root/obj.orig "$obj" && fsverity enable "$obj" && dropc
    cat "$victim" > /dev/null && echo "restored ok"
  } > "$out/tamper.txt" 2>&1
  cat "$out/tamper.txt" | grep -E 'tamper test|FAILED|restored'
fi
journalctl -b -k > "$out/dmesg.txt" 2>&1
echo "report written to $out"
