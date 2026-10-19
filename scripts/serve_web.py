#!/usr/bin/env python3
"""Serve the web build (from `scripts/build_web.py`) on localhost with the
cross-origin isolation headers its shared memory needs.

    python3 scripts/serve_web.py                     # target/web on http://localhost:8000
    python3 scripts/serve_web.py dist --port 8080

Browsers treat localhost as a secure page, so the audio worklet, the
microphone, and MIDI work as they do over https. For testing only: it's
Python's simple server.
"""

from __future__ import annotations

import argparse
import functools
import http.server
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
        ".mjs": "text/javascript",
        ".json": "application/json",
        ".acproj": "application/json",
        ".acblock": "application/json",
        ".wav": "audio/wav",
    }

    def end_headers(self) -> None:
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()

    def log_message(self, fmt: str, *args) -> None:  # quieter: errors only
        if args and str(args[1]).startswith(("4", "5")):
            super().log_message(fmt, *args)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dir", nargs="?", type=Path, default=ROOT / "target" / "web")
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--host", default="127.0.0.1")
    args = ap.parse_args()
    if not (args.dir / "index.html").is_file():
        raise SystemExit(f"{args.dir} has no index.html: build it with scripts/build_web.py")
    handler = functools.partial(Handler, directory=str(args.dir))
    with http.server.ThreadingHTTPServer((args.host, args.port), handler) as httpd:
        print(f"Serving {args.dir} at http://localhost:{args.port}/ (Ctrl+C stops)", flush=True)
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
