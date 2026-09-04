#!/usr/bin/env python3
"""Small persistent Blossom protocol fixture for native end-to-end tests."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import threading
from typing import Optional
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


MAX_BLOB_BYTES = 64 * 1024 * 1024
BLOB_PATH = re.compile(r"/([0-9a-f]{64})\.bin")


class BlossomFixtureServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(
        self,
        address: tuple[str, int],
        storage_dir: Path,
        request_log: Optional[Path] = None,
    ):
        super().__init__(address, BlossomFixtureHandler)
        self.storage_dir = storage_dir
        self.request_log = request_log
        self.request_log_lock = threading.Lock()
        self.request_sequence = 0

    def record_request(self, method: str, path: str, status: int) -> None:
        if self.request_log is None:
            return
        with self.request_log_lock:
            self.request_sequence += 1
            record = {
                "sequence": self.request_sequence,
                "method": method,
                "path": path,
                "status": status,
            }
            with self.request_log.open("a", encoding="utf-8") as output:
                output.write(json.dumps(record, separators=(",", ":")) + "\n")


class BlossomFixtureHandler(BaseHTTPRequestHandler):
    server: BlossomFixtureServer
    protocol_version = "HTTP/1.1"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self._path() == "/health":
            self._respond(200, b"ok\n", "text/plain")
            return
        self._serve_blob(include_body=True)

    def do_HEAD(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self._path() == "/health":
            self._respond(200, b"ok\n", "text/plain", include_body=False)
            return
        self._serve_blob(include_body=False)

    def do_PUT(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self._path() != "/upload":
            self._respond(404, b"not found\n", "text/plain")
            return
        try:
            content_length = int(self.headers.get("content-length", ""))
        except ValueError:
            self._respond(411, b"content-length required\n", "text/plain")
            return
        if not 0 <= content_length <= MAX_BLOB_BYTES:
            self._respond(413, b"blob too large\n", "text/plain")
            return
        body = self.rfile.read(content_length)
        if len(body) != content_length:
            self._respond(400, b"incomplete body\n", "text/plain")
            return
        digest = hashlib.sha256(body).hexdigest()
        expected = self.headers.get("x-sha-256")
        if expected is not None and expected.lower() != digest:
            self._respond(400, b"hash mismatch\n", "text/plain")
            return
        blob_path = self.server.storage_dir / f"{digest}.bin"
        try:
            descriptor = os.open(blob_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        except FileExistsError:
            self._respond(409, b"already exists\n", "text/plain")
            return
        with os.fdopen(descriptor, "wb") as blob:
            blob.write(body)
        self._respond(201, b"created\n", "text/plain")

    def _path(self) -> str:
        return urlsplit(self.path).path

    def _serve_blob(self, *, include_body: bool) -> None:
        match = BLOB_PATH.fullmatch(self._path())
        if match is None:
            self._respond(404, b"not found\n", "text/plain", include_body=include_body)
            return
        try:
            body = (self.server.storage_dir / f"{match.group(1)}.bin").read_bytes()
        except FileNotFoundError:
            self._respond(404, b"not found\n", "text/plain", include_body=include_body)
            return
        self._respond(200, body, "application/octet-stream", include_body=include_body)

    def _respond(
        self,
        status: int,
        body: bytes,
        content_type: str,
        *,
        include_body: bool = True,
    ) -> None:
        self.server.record_request(self.command, self._path(), status)
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        if include_body:
            self.wfile.write(body)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--storage-dir", type=Path, required=True)
    parser.add_argument("--ready-file", type=Path, required=True)
    parser.add_argument("--request-log", type=Path)
    return parser.parse_args()


def write_ready_file(path: Path, url: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(f"{url}\n", encoding="utf-8")
    os.replace(temporary, path)


def main() -> int:
    args = parse_args()
    if not 0 <= args.port <= 65535:
        raise SystemExit("--port must be between 0 and 65535")
    args.storage_dir.mkdir(parents=True, exist_ok=True)
    if args.request_log is not None:
        args.request_log.parent.mkdir(parents=True, exist_ok=True)
        args.request_log.write_text("", encoding="utf-8")
    server = BlossomFixtureServer(
        (args.host, args.port), args.storage_dir, args.request_log
    )
    host, port = server.server_address[:2]
    ready_file = args.ready_file.resolve()
    write_ready_file(ready_file, f"http://{host}:{port}")

    def stop_server(_signum: int, _frame: object) -> None:
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, stop_server)
    signal.signal(signal.SIGINT, stop_server)
    try:
        server.serve_forever(poll_interval=0.1)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        ready_file.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
