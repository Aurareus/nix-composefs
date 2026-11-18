#!/usr/bin/env bash
# nixcfs-hwtest: guided real-hardware test.  Run in a terminal (Konsole):
#   sudo nixcfs-hwtest 1   # after first login: measurements, tamper check, rebuild to gen 2
#   (reboot)
#   sudo nixcfs-hwtest 2   # checks gen 2 is booted and verified, sets one-shot rollback to gen 1
#   (reboot)
#   sudo nixcfs-hwtest 3   # checks the rollback, writes the final report
# Everything is logged to /boot/nixcfs-reports/ (the stick's FAT partition).
set -uo pipefail
set +o errexit   # (writeShellApplication turns it on; failures are recorded, not fatal)
if [ "$(id -u)" != 0 ]; then exec sudo "$0" "$@"; fi
R=/boot/nixcfs-reports
mkdir -p "$R"
log="$R/hwtest.log"
exec > >(tee -a "$log") 2>&1
say() { echo; echo "=== $(date -u +%FT%TZ) $*"; }
ok() { echo "PASS: $*"; }
bad() { echo "FAIL: $*"; FAILED=1; }
FAILED=0
GEN2=$(cat /etc/nixcfs-hwtest/gen2 2>/dev/null || true)
check_store() {
  d=$(grep -o 'nixcfs.image=[0-9a-f]*' /proc/cmdline | cut -d= -f2)
  if grep -q " /nix/store .*composefs:$d .*verity=require" /proc/self/mountinfo; then ok "store is composefs image $d with verity=require"; else bad "store not mounted as expected"; grep ' /nix/store ' /proc/self/mountinfo; fi
  [ "$(cat /run/nixcfs/image)" = "$d" ] && ok "initrd mounted the cmdline image" || bad "booted image mismatch"
}
case "${1:-}" in
1)
  say "stage 1 on $(hostname): $(tr -d '\0' < /sys/class/dmi/id/product_name 2>/dev/null)"
  check_store
  nixcfs status
  say "measurements (takes a few minutes; keep the machine idle)"
  nixcfs-bench "$R/bench-$(date -u +%Y%m%dT%H%M%SZ)"
  say "rebuild to generation 2 (from the bundle on the stick)"
  t0=$(date +%s.%N)
  nix-store --import < /var/lib/nixcfs-hwtest/gen2.nixexport > /dev/null || bad "importing gen 2"
  t1=$(date +%s.%N)
  nix-env -p /nix/var/nix/profiles/system --set "$GEN2" && "$GEN2/bin/switch-to-configuration" switch || bad "switch to gen 2"
  t2=$(date +%s.%N)
  echo "fetch $(echo "$t1-$t0"|bc) s, switch (incl. seal) $(echo "$t2-$t1"|bc) s"
  cp /nix/.cfs-rw/last-update-boot.json "$R/update-boot-gen2.json" 2>/dev/null
  [ "$(cat /etc/nixcfs-generation)" = 2 ] && ok "running gen 2" || bad "not running gen 2"
  nixcfs status
  say "stage 1 done: now REBOOT, log in, and run: sudo nixcfs-hwtest 2"
  ;;
2)
  say "stage 2"
  check_store
  [ "$(cat /etc/nixcfs-generation)" = 2 ] && ok "booted gen 2" || bad "did not boot gen 2"
  [ -e "/nix/.cfs-rw/upper/$(basename "$(readlink -f /run/current-system)")" ] && bad "gen 2 still in writable layer" || ok "gen 2 served from the verified image"
  systemd-analyze
  nix-store --verify --check-contents > /dev/null 2>&1 && ok "nix-store --verify --check-contents" || bad "verify"
  bootctl set-oneshot nixos-generation-1.conf && ok "one-shot boot of gen 1 set"
  say "stage 2 done: now REBOOT (do not pick anything in the menu), log in, and run: sudo nixcfs-hwtest 3"
  ;;
3)
  say "stage 3"
  check_store
  [ "$(cat /etc/nixcfs-generation)" = 1 ] && ok "rolled back to gen 1 via the boot menu" || bad "not gen 1"
  systemd-analyze
  journalctl -b -o short-monotonic -u nixcfs-store.service | tail -20
  say "all stages done. Shut down, and send back $R (or plug the stick into the build host)."
  ;;
*) echo "usage: nixcfs-hwtest 1|2|3"; exit 2;;
esac
[ $FAILED = 0 ] && echo "stage ${1} result: ALL PASS" || echo "stage ${1} result: SOME CHECKS FAILED"
sync
