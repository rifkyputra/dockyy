#!/usr/bin/env bash
# kuadrat phase-1 real-host acceptance.
# Run as root:  sudo bash kuadrat-acceptance.sh
# Expects the release binary to already be built (as your normal user).

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
SPEC=/tmp/kuadrat-acceptance-spec.json
NAME=acceptance-demo
SLUG=acceptance-demo
UNIT=kuadrat-${SLUG}
UNIT_FILE=/etc/containers/systemd/${UNIT}.container

pass=0; fail=0
ok()   { echo "  PASS  $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL  $1"; fail=$((fail+1)); }
step() { echo; echo "== $1"; }

[ -x "$BIN" ] || { echo "FATAL: $BIN not found. Build it first as your normal user:"; \
                   echo "  cd ~/devbox/kuadrat && PATH=\$HOME/.cargo/bin:\$PATH cargo build --release"; exit 1; }

echo "kuadrat phase-1 acceptance"
echo "binary : $BIN"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"
echo "unit   : $UNIT_FILE"

cat > "$SPEC" <<'EOF'
{
  "name": "acceptance-demo",
  "image": "docker.io/library/alpine:3",
  "command": ["sh", "-c", "echo kuadrat acceptance running; sleep 3600"],
  "env": [["GREETING", "hello world"]],
  "ports": [],
  "volumes": [],
  "secrets": [],
  "memory_max": "128M",
  "health_cmd": null,
  "restart_policy": "Always"
}
EOF

step "0. Pre-pull the image (avoids a slow first start)"
podman pull -q docker.io/library/alpine:3 >/dev/null 2>&1 && ok "image pulled" || bad "image pull"

step "1. apply"
"$BIN" apply "$SPEC"; rc=$?
[ $rc -eq 0 ] && ok "apply exited 0" || bad "apply exited $rc"

[ -f "$UNIT_FILE" ] && ok "unit file written at $UNIT_FILE" || bad "unit file missing"
grep -q '^# kuadrat-managed: true' "$UNIT_FILE" 2>/dev/null && ok "managed marker present" || bad "managed marker missing"
grep -q 'MemoryMax=128M' "$UNIT_FILE" 2>/dev/null && ok "MemoryMax rendered" || bad "MemoryMax missing"
# I1 regression: the spaced argument must be quoted, not split
grep -q 'Exec=sh -c "echo kuadrat acceptance running; sleep 3600"' "$UNIT_FILE" 2>/dev/null \
  && ok "I1: Exec= argument correctly quoted" || { bad "I1: Exec= quoting wrong"; grep '^Exec=' "$UNIT_FILE"; }

step "2. systemd sees it"
sleep 2
systemctl is-active --quiet "$UNIT" && ok "systemctl reports active" || { bad "unit not active"; systemctl status "$UNIT" --no-pager -l | head -20; }

step "3. podman sees the container"
podman ps --filter "name=kuadrat-${SLUG}" --format '{{.Names}} {{.Status}}' | grep -q . \
  && ok "container running: $(podman ps --filter name=kuadrat-${SLUG} --format '{{.Names}} {{.Status}}')" \
  || { bad "no container"; podman ps -a --filter name=kuadrat-${SLUG}; }

step "4. kuadrat's own read commands"
out=$("$BIN" list 2>&1); echo "$out" | grep -qx "$NAME" && ok "list shows $NAME" || bad "list output: $out"
out=$("$BIN" status "$NAME" 2>&1); [ "$out" = "Running" ] && ok "status = Running" || bad "status = $out"

step "5. C1 regression: refuse to touch a unit kuadrat does not own"
FOREIGN=/etc/containers/systemd/kuadrat-foreigner.container
printf '[Container]\nImage=alpine\n' > "$FOREIGN"
cat > /tmp/kuadrat-foreign-spec.json <<'EOF'
{"name":"foreigner","image":"docker.io/library/alpine:3","command":null,"env":[],"ports":[],
 "volumes":[],"secrets":[],"memory_max":null,"health_cmd":null,"restart_policy":"Always"}
EOF
"$BIN" apply /tmp/kuadrat-foreign-spec.json >/dev/null 2>&1
if grep -q '^# kuadrat-managed' "$FOREIGN"; then bad "C1: kuadrat OVERWROTE a foreign unit"; else ok "C1: refused to overwrite foreign unit"; fi
"$BIN" remove foreigner >/dev/null 2>&1
if [ -f "$FOREIGN" ]; then ok "C1: refused to delete foreign unit"; else bad "C1: kuadrat DELETED a foreign unit"; fi
rm -f "$FOREIGN" /tmp/kuadrat-foreign-spec.json

step "6. C2 regression: reject an injecting spec"
cat > /tmp/kuadrat-inject-spec.json <<'EOF'
{"name":"injector","image":"docker.io/library/alpine:3","command":null,
 "env":[["EVIL","1\nUser=root"]],"ports":[],"volumes":[],"secrets":[],
 "memory_max":null,"health_cmd":null,"restart_policy":"Always"}
EOF
if "$BIN" apply /tmp/kuadrat-inject-spec.json >/dev/null 2>&1; then
  bad "C2: injecting spec was ACCEPTED"; grep -n 'User=root' /etc/containers/systemd/kuadrat-injector.container 2>/dev/null
else
  ok "C2: injecting spec rejected"
fi
rm -f /tmp/kuadrat-inject-spec.json

step "7. remove"
"$BIN" remove "$NAME"; rc=$?
[ $rc -eq 0 ] && ok "remove exited 0" || bad "remove exited $rc"
[ ! -f "$UNIT_FILE" ] && ok "unit file gone" || bad "unit file still present"
sleep 2
podman ps --filter "name=kuadrat-${SLUG}" --format '{{.Names}}' | grep -q . \
  && bad "container still running" || ok "container stopped"

step "RESULT"
echo "  passed: $pass    failed: $fail"
[ $fail -eq 0 ] && echo "  PHASE 1 ACCEPTANCE: PASS" || echo "  PHASE 1 ACCEPTANCE: FAIL"

# Leave nothing behind
"$BIN" remove "$NAME" >/dev/null 2>&1
rm -f "$SPEC"
systemctl daemon-reload
exit $fail
