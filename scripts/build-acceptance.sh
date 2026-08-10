#!/usr/bin/env bash
# kuadrat G2 build acceptance. Run as your normal user (podman rootless is fine):
#   bash scripts/build-acceptance.sh
# Expects the release binary built:  cargo build --release

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APP=g2demo
SLUG=g2demo
IMAGE="localhost/kuadrat-${SLUG}"

pass=0; fail=0
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); }

[ -x "$BIN" ] || { echo "FATAL: $BIN not found. Build it: PATH=\$HOME/.cargo/bin:\$PATH cargo build --release"; exit 1; }

echo "kuadrat G2 build acceptance"
echo "binary : $BIN"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"
echo "workdir: $WORK/${APP}"

# A tiny git repo with a Containerfile.
mkdir -p "$WORK/$APP"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
RUN echo "kuadrat g2 build" > /built.txt
EOF
git -C "$WORK/$APP" init -q
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t add -A
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -qm "init"
SHA=$(git -C "$WORK/$APP" rev-parse HEAD)

echo "== build"
OUT=$("$BIN" build "$WORK/$APP" 2>&1); rc=$?
echo "$OUT"
[ $rc -eq 0 ] && ok "build exited 0" || bad "build exited $rc"
[ "$OUT" = "${IMAGE}:${SHA}" ] && ok "printed reference matches localhost/kuadrat-<slug>:<sha>" || bad "reference was '$OUT', expected '${IMAGE}:${SHA}'"

echo "== podman sees the image"
podman image exists "${IMAGE}:${SHA}" && ok "image ${IMAGE}:${SHA} exists" || bad "image not found"

echo "== build with a relative '.' path"
OUT2=$( cd "$WORK/$APP" && "$BIN" build . 2>&1 ); rc2=$?
echo "$OUT2"
[ $rc2 -eq 0 ] && ok "build . exited 0" || bad "build . exited $rc2"
[ "$OUT2" = "${IMAGE}:${SHA}" ] && ok "build . printed reference matches localhost/kuadrat-<slug>:<sha>" || bad "build . reference was '$OUT2', expected '${IMAGE}:${SHA}'"

echo "== detect rejects a non-repo"
mkdir -p "$WORK/norepo"
cp "$WORK/$APP/Containerfile" "$WORK/norepo/"
"$BIN" build "$WORK/norepo" >/dev/null 2>&1 && bad "build of a non-repo should fail" || ok "build of a non-repo fails"

echo "== detect rejects a repo with no Containerfile"
mkdir -p "$WORK/nocf"; git -C "$WORK/nocf" init -q
"$BIN" build "$WORK/nocf" >/dev/null 2>&1 && bad "build with no Containerfile should fail" || ok "build with no Containerfile fails"

echo "== RESULT"
echo "  passed: $pass    failed: $fail"

# Clean up the built image and the workdir.
podman rmi -f "${IMAGE}:${SHA}" >/dev/null 2>&1
rm -rf "$WORK"
[ $fail -eq 0 ] && echo "  G2 BUILD ACCEPTANCE: PASS" || echo "  G2 BUILD ACCEPTANCE: FAIL"
exit $fail
