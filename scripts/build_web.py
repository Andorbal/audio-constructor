#!/usr/bin/env python3
"""Build Audio Constructor for the browser: a static site.

    python3 scripts/build_web.py                 # release build into target/web/
    python3 scripts/build_web.py --serve         # ... then serve it at http://localhost:8000
    python3 scripts/build_web.py --debug         # a debug build (slower, with debug checks)
    python3 scripts/build_web.py --out dist      # somewhere else
    python3 scripts/build_web.py --no-build      # assemble the site from what's built

The site holds:

- `index.html`, `boot.js`: the page, which checks for cross-origin
  isolation (shared memory needs it) and starts the editor.
- `ac_web.js`, `ac_web_bg.wasm`, `snippets/`: the editor (the `ui` build of
  `ac-web`, through wasm-bindgen).
- `ac_audio.wasm`, `worklet.js`: the audio worklet and the engine it runs
  (the `worklet` build of `ac-web`, used as it is).
- `sw.js`: a service worker that adds the isolation headers on hosts that
  can't, and `_headers` for hosts that read one.
- `examples/` with `index.json`, and `blocks/`: the shipped examples (the
  Examples menu fetches them) and blocks (built into the editor too).

Serve it from any static host that sends COOP and COEP (see `_headers`),
or any host at all thanks to `sw.js`. `scripts/serve_web.py` serves it
locally with the headers.

Needs Python 3.8 or newer, cargo with the wasm32-unknown-unknown target
(rust-toolchain.toml installs it), and wasm-bindgen's command line at the
version in Cargo.lock:

    cargo install wasm-bindgen-cli --version <that version> --locked

`wasm-opt` (binaryen) is used to shrink both modules when it's installed.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATE = "ac-web"
LIB = "ac_web"
TARGET = "wasm32-unknown-unknown"
WEB = ROOT / "crates" / "ac-web" / "web"
STATIC = ["index.html", "boot.js", "worklet.js", "sw.js", "_headers"]


def run(cmd: list[str], env: dict[str, str] | None = None) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, check=True, env=env)


def target_dir() -> Path:
    env = os.environ.get("CARGO_TARGET_DIR")
    return Path(env) if env else ROOT / "target"


def locked_version(crate: str) -> str | None:
    text = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    m = re.search(r'\[\[package\]\]\nname = "' + re.escape(crate) + r'"\nversion = "([^"]+)"', text)
    return m.group(1) if m else None


def check_bindgen() -> str:
    want = locked_version("wasm-bindgen")
    exe = shutil.which("wasm-bindgen")
    hint = f"cargo install wasm-bindgen-cli --version {want} --locked"
    if not exe:
        sys.exit(f"wasm-bindgen isn't installed; install the version Cargo.lock uses:\n    {hint}")
    out = subprocess.run([exe, "--version"], capture_output=True, text=True, check=True).stdout
    have = out.split()[-1] if out.split() else "?"
    if want and have != want:
        sys.exit(
            f"wasm-bindgen {have} is installed, but Cargo.lock uses {want}; install that one:\n"
            f"    {hint}"
        )
    return exe


def build(profile: str, features: str) -> Path:
    cmd = ["cargo", "build", "-p", CRATE, "--target", TARGET, "--no-default-features", "--features", features]
    if profile == "release":
        cmd.append("--release")
    # No debug info in what's served: it would make the modules several
    # times bigger. The editor keeps function names for its panic messages.
    env = dict(os.environ)
    env.setdefault(f"CARGO_PROFILE_{profile.upper()}_DEBUG", "false")
    run(cmd, env)
    return target_dir() / TARGET / ("release" if profile == "release" else "debug") / f"{LIB}.wasm"


def optimize(path: Path) -> None:
    exe = shutil.which("wasm-opt")
    if not exe:
        return
    run([exe, "-O2", "--enable-bulk-memory", "--enable-nontrapping-float-to-int",
         "--enable-sign-ext", "--enable-mutable-globals", str(path), "-o", str(path)])


def strip_custom_sections(path: Path) -> None:
    """Drop a raw module's custom sections (debug info, producers): the
    worklet's module goes out as cargo built it, with no wasm-bindgen pass."""
    data = path.read_bytes()
    if data[:4] != b"\0asm":
        sys.exit(f"{path} isn't a wasm module")
    out = bytearray(data[:8])
    i = 8
    while i < len(data):
        sid = data[i]
        i += 1
        size = 0
        shift = 0
        while True:
            b = data[i]
            i += 1
            size |= (b & 0x7F) << shift
            shift += 7
            if b < 0x80:
                break
        start = i
        i += size
        if sid == 0:
            continue
        out.append(sid)
        n = size
        while True:
            b = n & 0x7F
            n >>= 7
            out.append(b | (0x80 if n else 0))
            if not n:
                break
        out += data[start:i]
    path.write_bytes(bytes(out))


def fresh(path: Path) -> None:
    if path.is_dir():
        shutil.rmtree(path)
    path.mkdir(parents=True)


def site(out: Path, profile: str, do_build: bool) -> None:
    bindgen = check_bindgen()
    sub = "release" if profile == "release" else "debug"
    built = target_dir() / TARGET / sub / f"{LIB}.wasm"
    staging = target_dir() / "web-staging"
    staging.mkdir(parents=True, exist_ok=True)
    audio = staging / "ac_audio.wasm"
    if do_build:
        # The worklet's module first: both builds write the same file.
        shutil.copy2(build(profile, "worklet"), audio)
        build(profile, "ui")
    elif not audio.exists():
        sys.exit(f"nothing built yet: {audio} is missing (run without --no-build)")
    fresh(out)
    run([bindgen, "--target", "web", "--no-typescript", "--out-dir", str(out), "--out-name", LIB, str(built)])
    shutil.copy2(audio, out / "ac_audio.wasm")
    strip_custom_sections(out / "ac_audio.wasm")
    optimize(out / "ac_audio.wasm")
    optimize(out / f"{LIB}_bg.wasm")
    for name in STATIC:
        shutil.copy2(WEB / name, out / name)
    examples = out / "examples"
    examples.mkdir()
    names = sorted(p.name for p in (ROOT / "examples").glob("*.acproj"))
    for name in names:
        shutil.copy2(ROOT / "examples" / name, examples / name)
    if (ROOT / "examples" / "assets").is_dir():
        shutil.copytree(ROOT / "examples" / "assets", examples / "assets")
    (examples / "index.json").write_text(json.dumps({"examples": names}, indent=2) + "\n", encoding="utf-8")
    blocks = out / "blocks"
    blocks.mkdir()
    for p in sorted((ROOT / "blocks").glob("*.acblock")):
        shutil.copy2(p, blocks / p.name)
    total = sum(f.stat().st_size for f in out.rglob("*") if f.is_file())
    print(f"\nThe site is in {out} ({total / 1e6:.1f} MB):")
    for f in ["ac_web_bg.wasm", "ac_audio.wasm"]:
        print(f"  {f:16} {(out / f).stat().st_size / 1e6:6.1f} MB")
    print(f"  {len(names)} examples, {len(list(blocks.iterdir()))} blocks")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--debug", action="store_true", help="a debug build")
    ap.add_argument("--out", type=Path, default=None, help="where the site goes (default target/web)")
    ap.add_argument("--no-build", action="store_true", help="assemble the site from what's built")
    ap.add_argument("--serve", action="store_true", help="then serve it with the isolation headers")
    ap.add_argument("--port", type=int, default=8000)
    args = ap.parse_args()
    out = (args.out or target_dir() / "web").resolve()
    site(out, "debug" if args.debug else "release", not args.no_build)
    if args.serve:
        run([sys.executable, str(ROOT / "scripts" / "serve_web.py"), str(out), "--port", str(args.port)])


if __name__ == "__main__":
    main()
