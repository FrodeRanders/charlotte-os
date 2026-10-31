#!/usr/bin/env python3
"""Bounded local HTTP fault receiver for the native OTLP exporter qualification.

The mode file selects success, partial, retry, stall, oversized or malformed.
Use the digest-pinned official Collector for protocol interoperability; this
fixture tests failure isolation and records request digests/timestamps.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import socket
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--mode-file", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    args = parser.parse_args()
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            pass

        def do_POST(self):
            self.connection.settimeout(3)
            try:
                length = int(self.headers.get("Content-Length", "-1"))
                if (self.path != "/v1/metrics" or not 0 < length <= 8192
                        or self.headers.get("Content-Type") != "application/x-protobuf"):
                    self.send_error(400)
                    return
                body = self.rfile.read(length)
                if len(body) != length:
                    return
                mode = args.mode_file.read_text().strip()
                with lock, args.log.open("a") as output:
                    output.write(json.dumps({"monotonic": time.monotonic(), "mode": mode,
                                             "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()}) + "\n")
                if mode == "stall":
                    time.sleep(3)
                    self.close_connection = True
                    return
                status = 503 if mode == "retry" else 200
                data = {"success": b"\x0a\x00", "partial": b"\x0a\x02\x08\x01",
                        "retry": b"", "oversized": b"", "malformed": b"\x0a\xff"}.get(mode)
                if data is None:
                    self.send_error(400)
                    return
                self.send_response(status)
                self.send_header("Content-Type", "application/x-protobuf")
                self.send_header("Content-Length", str(8193 if mode == "oversized" else len(data)))
                self.send_header("Connection", "close")
                if mode == "retry":
                    self.send_header("Retry-After", "5")
                self.end_headers()
                self.wfile.write(data)
                self.close_connection = True
            except (OSError, ValueError, socket.timeout):
                self.close_connection = True

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"OTLP fault fixture listening on 127.0.0.1:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
