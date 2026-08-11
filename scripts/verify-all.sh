#!/usr/bin/env bash
# Run every acceptance script in order and print one verdict.
#
#   PATH=$HOME/.cargo/bin:$PATH cargo build --release   # as your normal user
#   sudo bash scripts/verify-all.sh
#
# Each script is self-cleaning and independent; a failure in one does not stop
# the rest, so a single run tells you everything that is broken, not just the
# first thing.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
BIN=$PWD/target/release/kuadrat

[ -x "$BIN" ] || { echo "FATAL: $BIN not found — build it as your user first"; exit 1; }
[ "$(id -u)" -eq 0 ] || { echo "FATAL: needs root (system Quadlet units) — use sudo"; exit 1; }

SCRIPTS=(
  acceptance.sh
  build-acceptance.sh
  secrets-acceptance.sh
  deploy-acceptance.sh
  reconcile-acceptance.sh
  serve-acceptance.sh
)

declare -a NAMES=() RESULTS=()
worst=0

for s in "${SCRIPTS[@]}"; do
  echo
  echo "################ $s"
  bash "scripts/$s"
  rc=$?
  NAMES+=("$s")
  if [ $rc -eq 0 ]; then RESULTS+=("PASS"); else RESULTS+=("FAIL($rc)"); worst=1; fi
done

echo
echo "################ SUMMARY"
for i in "${!NAMES[@]}"; do
  printf '  %-26s %s\n' "${NAMES[$i]}" "${RESULTS[$i]}"
done
echo
[ $worst -eq 0 ] && echo "  ALL ACCEPTANCE: PASS" || echo "  ALL ACCEPTANCE: FAIL"
exit $worst
