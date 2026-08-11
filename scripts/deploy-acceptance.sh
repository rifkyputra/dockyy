#!/usr/bin/env bash
# kuadrat G4b deploy acceptance. Needs root (system Quadlet units):
#   sudo bash scripts/deploy-acceptance.sh
# Build the binary first (as your normal user):
#   PATH=$HOME/.cargo/bin:$PATH cargo build --release

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
APP=g4bdemo
SLUG=g4bdemo
UNIT=kuadrat-${SLUG}
WORK=$(mktemp -d)

pass=0; fail=0
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); }
cleanup() {
  "$BIN" remove "$APP" >/dev/null 2>&1
  rm -rf "$WORK"
  systemctl daemon-reload
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "FATAL: $BIN not found — build it as your user first"; exit 1; }
[ "$(id -u)" -eq 0 ] || { echo "FATAL: run as root (sudo) — system Quadlet units need it"; exit 1; }

echo "kuadrat G4b deploy acceptance"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"

# A fixture repo: a working app that stays up, plus a kuadrat.json.
mkdir -p "$WORK/$APP"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
CMD ["sh", "-c", "echo v1 up; sleep 3600"]
EOF
cat > "$WORK/$APP/kuadrat.json" <<'EOF'
{"name":"g4bdemo","image":"","command":null,"env":[],"ports":[],"volumes":[],
 "secrets":[],"memory_max":"128M","health_cmd":null,"restart_policy":"Always","route":null}
EOF
git -C "$WORK/$APP" init -q
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t add -A
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -qm v1

echo "== deploy v1"
OUT=$("$BIN" deploy "$APP" "$WORK/$APP" 2>&1); echo "$OUT"
echo "$OUT" | grep -q 'Done' && ok "deploy v1 -> Done" || bad "deploy v1 did not reach Done"
systemctl is-active --quiet "$UNIT" && ok "v1 unit active" || bad "v1 unit not active"

echo "== deploy a broken commit (bad Containerfile) and expect rollback"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
RUN exit 1
EOF
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -aqm broken
OUT2=$("$BIN" deploy "$APP" "$WORK/$APP" 2>&1); echo "$OUT2"
echo "$OUT2" | grep -qE 'RolledBack|Failed' && ok "broken deploy did not report Done" || bad "broken deploy unexpectedly reported Done"
# The v1 unit must still be present and active after the failed deploy.
systemctl is-active --quiet "$UNIT" && ok "v1 still active after the failed deploy" || bad "v1 was lost by the failed deploy"

echo "== remove"
"$BIN" remove "$APP" >/dev/null 2>&1
systemctl is-active --quiet "$UNIT" && bad "unit still active after remove" || ok "unit stopped after remove"

echo "== RESULT"
echo "  passed: $pass    failed: $fail"
[ $fail -eq 0 ] && echo "  G4B DEPLOY ACCEPTANCE: PASS" || echo "  G4B DEPLOY ACCEPTANCE: FAIL"
exit $fail
