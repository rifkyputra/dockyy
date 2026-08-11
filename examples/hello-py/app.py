"""A single-file HTTP service on port 7654, standard library only.

Deliberately dependency-free: the point of this example is to exercise
kuadrat's deploy loop, not to demonstrate a web framework. No pip install
means the build stage is fast and cannot fail on a network hiccup.
"""

import json
import os
import socket
import signal
import sys
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 7654
STARTED_AT = datetime.now(timezone.utc).isoformat()


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, payload):
        body = json.dumps(payload, indent=2).encode() + b"\n"
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/healthz":
            # Kept trivial on purpose: a healthcheck that can fail for reasons
            # unrelated to the app turns a deploy into a coin flip.
            self._send(200, {"status": "ok"})
        elif self.path == "/":
            self._send(
                200,
                {
                    "app": "hello-py",
                    "greeting": os.environ.get("GREETING", "hello from kuadrat"),
                    "host": socket.gethostname(),
                    "started_at": STARTED_AT,
                    "port": PORT,
                },
            )
        else:
            self._send(404, {"error": "not found", "path": self.path})

    def log_message(self, fmt, *args):
        # journald already stamps the time and unit, so the default
        # "127.0.0.1 - - [date]" prefix is duplicated noise in `journalctl`.
        sys.stderr.write("%s\n" % (fmt % args))


def shutdown(signum, _frame):
    # Without this, the container ignores SIGTERM and systemd waits out the
    # full stop timeout on every restart during a deploy.
    sys.stderr.write(f"received signal {signum}, shutting down\n")
    sys.exit(0)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)
    server = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    sys.stderr.write(f"listening on 0.0.0.0:{PORT}\n")
    server.serve_forever()
