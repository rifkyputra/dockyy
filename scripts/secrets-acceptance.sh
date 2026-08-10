#!/usr/bin/env bash
# kuadrat G3 secrets acceptance. Run as your normal user (rootless podman):
#   bash scripts/secrets-acceptance.sh
# Expects the release binary:  cargo build --release

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
NAME=kuadrat-g3-test
VALUE="top-secret-value-$$"

pass=0; fail=0
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); }
cleanup() { podman secret rm "$NAME" >/dev/null 2>&1; }
trap cleanup EXIT

[ -x "$BIN" ] || { echo "FATAL: $BIN not found. Build it: PATH=\$HOME/.cargo/bin:\$PATH cargo build --release"; exit 1; }

echo "kuadrat G3 secrets acceptance"
echo "binary : $BIN"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"

# Start clean.
podman secret rm "$NAME" >/dev/null 2>&1

echo "== set (value via stdin)"
printf %s "$VALUE" | "$BIN" secret set "$NAME"
rc=$?
[ $rc -eq 0 ] && ok "secret set exited 0" || bad "secret set exited $rc"

echo "== kuadrat and podman both see it"
"$BIN" secret ls | grep -qx "$NAME" && ok "kuadrat secret ls shows $NAME" || bad "kuadrat ls missing $NAME"
podman secret ls --format '{{.Name}}' | grep -qx "$NAME" && ok "podman secret ls shows $NAME" || bad "podman ls missing $NAME"

echo "== re-set (upsert) is idempotent"
printf %s "$VALUE-v2" | "$BIN" secret set "$NAME" && ok "re-set (upsert) exited 0" || bad "re-set failed"

echo "== rm"
"$BIN" secret rm "$NAME" >/dev/null && ok "secret rm exited 0" || bad "secret rm failed"
podman secret ls --format '{{.Name}}' | grep -qx "$NAME" && bad "secret still present after rm" || ok "secret gone after rm"

echo "== RESULT"
echo "  passed: $pass    failed: $fail"
[ $fail -eq 0 ] && echo "  G3 SECRETS ACCEPTANCE: PASS" || echo "  G3 SECRETS ACCEPTANCE: FAIL"
exit $fail
