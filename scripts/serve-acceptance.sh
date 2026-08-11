#!/usr/bin/env bash
# kuadrat H7 serve acceptance. Needs root (system Quadlet units + daemon-reload):
#   sudo bash scripts/serve-acceptance.sh
# Build the binary first (as your normal user):
#   PATH=$HOME/.cargo/bin:$PATH cargo build --release
#
# Exercises `kuadrat serve`: the loopback guard, the three web pages, the SSE
# stream, `kuadrat deploy`'s daemon handoff and its local fallback, and the
# webhook. Deploys a real (throwaway) workload and starts a real daemon on
# this host — that is the point, and why this is not run automatically.

set -uo pipefail

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BIN="$REPO/target/release/kuadrat"

APP=h7servedemo
SLUG=h7servedemo
UNIT=kuadrat-${SLUG}
PORT=17457
LISTEN=127.0.0.1:${PORT}
APP_PORT=18093
WORK=$(mktemp -d)
HOOKDIR=$(mktemp -d)
HOOKPORT=17458
HOOKLOG="$HOOKDIR/received.log"

DAEMON_PID=""
HOOK_PID=""

pass=0; fail=0
EVIDENCE_DIR=""
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); capture_evidence; }

# On a failing check, snapshot whatever diagnostic state still exists — the
# unit, the container, and the journal — into a directory `cleanup` does not
# touch. Without this, cleanup's `trap ... EXIT` removes the app and the temp
# dir on every exit, including a failing one, so twice now the evidence
# needed to diagnose a failure was gone before anyone could look. Re-running
# this on every `bad()` call (not just the first) keeps the snapshot current
# as later checks run, at the cost of overwriting earlier failures' captures
# with the same files — acceptable since the files describe live host state,
# not per-check history.
capture_evidence() {
  if [ -z "$EVIDENCE_DIR" ]; then
    EVIDENCE_DIR="/tmp/kuadrat-acceptance-$(date +%Y%m%dT%H%M%S)"
  fi
  mkdir -p "$EVIDENCE_DIR"
  cat "/etc/containers/systemd/${UNIT}.container" > "$EVIDENCE_DIR/unit.container" 2>&1
  systemctl status "$UNIT" > "$EVIDENCE_DIR/systemctl-status.txt" 2>&1
  journalctl -u "$UNIT" -n 100 --no-pager > "$EVIDENCE_DIR/journal.txt" 2>&1
  podman ps -a --filter "name=${UNIT}" > "$EVIDENCE_DIR/podman-ps.txt" 2>&1
}

cleanup() {
  [ -n "$DAEMON_PID" ] && kill "$DAEMON_PID" >/dev/null 2>&1
  [ -n "$HOOK_PID" ] && kill "$HOOK_PID" >/dev/null 2>&1
  [ -n "$DAEMON_PID" ] && wait "$DAEMON_PID" 2>/dev/null
  [ -n "$HOOK_PID" ] && wait "$HOOK_PID" 2>/dev/null
  "$BIN" remove "$APP" >/dev/null 2>&1
  rm -rf "$WORK" "$HOOKDIR"
  systemctl daemon-reload
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "FATAL: $BIN not found — build it as your user first"; exit 1; }
[ "$(id -u)" -eq 0 ] || { echo "FATAL: run as root (sudo) — the daemon writes system Quadlet units"; exit 1; }
for tool in podman curl git python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "FATAL: $tool not found on PATH"; exit 1; }
done

echo "kuadrat H7 serve acceptance"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"

# A fixture repo: a working app that stays up and answers HTTP on its own
# port, plus a kuadrat.json publishing that port.
mkdir -p "$WORK/$APP"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
RUN apk add --no-cache busybox-extras && mkdir -p /www && printf OK > /www/index.html
CMD ["httpd", "-f", "-p", "8000", "-h", "/www"]
EOF
cat > "$WORK/$APP/kuadrat.json" <<EOF
{"name":"$APP","image":"","command":null,"env":[],"ports":["${APP_PORT}:8000"],"volumes":[],
 "secrets":[],"memory_max":"128M","health_cmd":"wget -qO- http://127.0.0.1:8000/ | grep -q OK","restart_policy":"Always","route":null}
EOF
git -C "$WORK/$APP" init -q
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t add -A
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -qm v1

# --------------------------------------------------------------------------
echo "== 1: the loopback guard refuses a non-loopback --listen"
OUT1=$("$BIN" serve --listen 0.0.0.0:7457 2>&1); RC1=$?
echo "$OUT1"
[ "$RC1" -ne 0 ] && ok "serve on 0.0.0.0:7457 exited non-zero" || bad "serve on 0.0.0.0:7457 exited 0"
echo "$OUT1" | grep -q "ssh -L 7457:127.0.0.1:7457" && ok "the refusal names the SSH tunnel" || bad "the refusal did not name the tunnel"

# --------------------------------------------------------------------------
echo "== throwaway webhook receiver (never a real chat service)"
cat > "$HOOKDIR/receiver.py" <<'PYEOF'
import http.server, sys
log_path, port = sys.argv[1], int(sys.argv[2])

class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        with open(log_path, "ab") as f:
            f.write(body + b"\n")
        self.send_response(200)
        self.end_headers()

    def log_message(self, *args):
        pass

http.server.HTTPServer(("127.0.0.1", port), Handler).serve_forever()
PYEOF
: > "$HOOKLOG"
python3 "$HOOKDIR/receiver.py" "$HOOKLOG" "$HOOKPORT" &
HOOK_PID=$!

deadline=$((SECONDS + 10))
until curl -s -o /dev/null "http://127.0.0.1:${HOOKPORT}/" || [ "$SECONDS" -ge "$deadline" ]; do
  sleep 0.5
done
kill -0 "$HOOK_PID" >/dev/null 2>&1 || { echo "FATAL: the throwaway webhook receiver did not start"; exit 1; }

# --------------------------------------------------------------------------
echo "== 2: the daemon starts on loopback and GET / answers 200"
KUADRAT_WEBHOOK_URL="http://127.0.0.1:${HOOKPORT}/hook" "$BIN" serve --listen "$LISTEN" &
DAEMON_PID=$!

deadline=$((SECONDS + 20))
up=""
until [ "$SECONDS" -ge "$deadline" ]; do
  code=$(curl -s -o /dev/null -w '%{http_code}' "http://${LISTEN}/" 2>/dev/null)
  [ "$code" = "200" ] && { up=1; break; }
  kill -0 "$DAEMON_PID" >/dev/null 2>&1 || break
  sleep 0.5
done
if [ -n "$up" ]; then
  ok "GET / answered 200 once the daemon was up"
else
  echo "FATAL: the daemon never answered on $LISTEN"
  exit 1
fi

# --------------------------------------------------------------------------
echo "== 3: POST /apps registers an app and redirects"
REG=$(curl -s -D - -o /dev/null -X POST \
  --data-urlencode "name=$APP" \
  --data-urlencode "repo_path=$WORK/$APP" \
  "http://${LISTEN}/apps")
echo "$REG" | grep -qi '^HTTP/[0-9.]* 303' && ok "registration redirected (303)" || bad "registration did not redirect: $REG"
echo "$REG" | grep -qi "location:.*$APP" && ok "the redirect points at the app's page" || bad "no Location header naming $APP: $REG"

# --------------------------------------------------------------------------
echo "== 4: a deploy over the socket reaches Done, streaming six stages"
DEPLOY_RESP=$(curl -s -X POST -H 'Accept: application/json' "http://${LISTEN}/api/apps/$APP/deploy")
echo "$DEPLOY_RESP"
DEPLOY_ID=$(echo "$DEPLOY_RESP" | grep -oE '"deploy_id":[0-9]+' | grep -oE '[0-9]+')
if [ -z "$DEPLOY_ID" ]; then
  echo "FATAL: could not start a deploy over the socket: $DEPLOY_RESP"
  exit 1
fi

SSE=$(curl -sN --max-time 180 "http://${LISTEN}/api/deploys/${DEPLOY_ID}/events")
echo "$SSE" | grep -q '"status":"done"' && ok "deploy $DEPLOY_ID over the socket reached Done" || bad "deploy $DEPLOY_ID did not reach Done: $SSE"

missing=""
for stage in detect build secrets apply route healthcheck; do
  echo "$SSE" | grep -q "\"stage\":\"$stage\"" || missing="$missing $stage"
done
if [ -z "$missing" ]; then
  ok "the event stream carried all six stages"
else
  bad "the event stream was missing stages:$missing"
fi

# --------------------------------------------------------------------------
echo "== 9 (checked here, while the daemon that saw the event is still up): a webhook POST is attempted on the terminal event"
deadline=$((SECONDS + 15))
until [ -s "$HOOKLOG" ] || [ "$SECONDS" -ge "$deadline" ]; do
  sleep 0.5
done
if [ -s "$HOOKLOG" ]; then
  grep -q "\"app\": *\"$APP\"" "$HOOKLOG" && ok "the webhook receiver got a POST naming $APP" || bad "the webhook receiver's payload did not name $APP: $(cat "$HOOKLOG")"
  grep -q '"status": *"done"' "$HOOKLOG" && ok "the webhook payload reported the terminal status" || bad "the webhook payload did not report done: $(cat "$HOOKLOG")"
else
  bad "no request ever reached the throwaway webhook receiver"
fi

# --------------------------------------------------------------------------
echo "== 5: /app/:name and /deploy/:id render"
APP_CODE=$(curl -s -o "$WORK/app.html" -w '%{http_code}' "http://${LISTEN}/app/$APP")
[ "$APP_CODE" = "200" ] && grep -q "$APP" "$WORK/app.html" && ok "/app/$APP rendered" || bad "/app/$APP did not render (http $APP_CODE)"

DEPLOY_CODE=$(curl -s -o "$WORK/deploy.html" -w '%{http_code}' "http://${LISTEN}/deploy/${DEPLOY_ID}")
[ "$DEPLOY_CODE" = "200" ] && grep -q "Deploy ${DEPLOY_ID}" "$WORK/deploy.html" && ok "/deploy/${DEPLOY_ID} rendered" || bad "/deploy/${DEPLOY_ID} did not render (http $DEPLOY_CODE)"

# --------------------------------------------------------------------------
echo "== 6: the deployed app answers on its own port"
deadline=$((SECONDS + 20))
answered=""
until [ "$SECONDS" -ge "$deadline" ]; do
  curl -s "http://127.0.0.1:${APP_PORT}/" 2>/dev/null | grep -q OK && { answered=1; break; }
  sleep 1
done
[ -n "$answered" ] && ok "the deployed app answered on 127.0.0.1:${APP_PORT}" || bad "the deployed app never answered on 127.0.0.1:${APP_PORT}"

# --------------------------------------------------------------------------
echo "== 7: kuadrat deploy hands off to the daemon while it is running"
OUT7=$("$BIN" deploy "$APP" "$WORK/$APP" --listen "$LISTEN" 2>&1)
echo "$OUT7"
echo "$OUT7" | grep -q "queued as deploy" && echo "$OUT7" | grep -q "http://${LISTEN}/deploy/" \
  && ok "kuadrat deploy handed off to the running daemon" \
  || bad "kuadrat deploy did not hand off: $OUT7"

# Let the handed-off deploy settle before the daemon goes away, so it is not
# left in progress when the process is killed below.
ID7=$(echo "$OUT7" | grep -oE 'queued as deploy [0-9]+' | grep -oE '[0-9]+')
[ -n "$ID7" ] && curl -sN --max-time 180 "http://${LISTEN}/api/deploys/${ID7}/events" >/dev/null 2>&1

# --------------------------------------------------------------------------
echo "== 8: kuadrat deploy runs in-process once the daemon is stopped"
kill "$DAEMON_PID" >/dev/null 2>&1
wait "$DAEMON_PID" 2>/dev/null
DAEMON_PID=""

deadline=$((SECONDS + 10))
until ! curl -s -o /dev/null "http://${LISTEN}/" 2>/dev/null || [ "$SECONDS" -ge "$deadline" ]; do
  sleep 0.5
done

OUT8=$(timeout 180 "$BIN" deploy "$APP" "$WORK/$APP" --listen "$LISTEN" 2>&1)
echo "$OUT8"
echo "$OUT8" | grep -q "no daemon running; deploying locally" && echo "$OUT8" | grep -q 'Done' \
  && ok "kuadrat deploy fell back to an in-process run and reached Done" \
  || bad "kuadrat deploy did not fall back cleanly: $OUT8"

# --------------------------------------------------------------------------
echo "== RESULT"
echo "  passed: $pass    failed: $fail"
if [ $fail -eq 0 ]; then
  echo "  H7 SERVE ACCEPTANCE: PASS"
else
  echo "  H7 SERVE ACCEPTANCE: FAIL"
  [ -n "$EVIDENCE_DIR" ] && echo "  evidence captured in: $EVIDENCE_DIR"
fi
exit $fail
