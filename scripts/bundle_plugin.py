#!/usr/bin/env python3
"""Build the plugin and wrap it in `.clap` and `.vst3` bundles.

One library holds both plugins (Audio Constructor Effect and Audio
Constructor Instrument), so there is one bundle per format:

    python3 scripts/bundle_plugin.py                 # release build into target/bundled/
    python3 scripts/bundle_plugin.py --install       # ... and copy into your plugin folders
    python3 scripts/bundle_plugin.py --debug         # a debug build (aborts on audio-thread allocations)
    python3 scripts/bundle_plugin.py --target aarch64-apple-darwin --out dist
    python3 scripts/bundle_plugin.py --no-build      # bundle what's already built

Layouts (what hosts look for):

- Linux:   `Audio Constructor.clap` (the shared library itself) and
           `Audio Constructor.vst3/Contents/<arch>-linux/Audio Constructor.so`
- macOS:   `Audio Constructor.clap` and `Audio Constructor.vst3`, each a
           bundle with `Contents/Info.plist`, `Contents/PkgInfo`, and
           `Contents/MacOS/Audio Constructor`
- Windows: `Audio Constructor.clap` (the DLL) and
           `Audio Constructor.vst3/Contents/<arch>-win/Audio Constructor.vst3`

Needs Python 3.8 or newer and cargo; nothing else. The bundles aren't
signed: on macOS, clear the quarantine flag on downloaded copies with
`xattr -dr com.apple.quarantine <bundle>`.
"""

from __future__ import annotations

import argparse
import os
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NAME = "Audio Constructor"
BUNDLE_ID = "io.github.andorbal.audio-constructor"
CRATE = "ac-plugin"
LIB = "ac_plugin"


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"', text, re.M | re.S)
    return m.group(1) if m else "0.0.0"


def host_os(target: str | None) -> str:
    """'linux', 'macos', or 'windows', from the target triple or this machine."""
    probe = target or sys.platform
    if "apple" in probe or "darwin" in probe:
        return "macos"
    if "windows" in probe or probe.startswith("win"):
        return "windows"
    return "linux"


def arch(target: str | None) -> str:
    raw = (target.split("-")[0] if target else platform.machine()).lower()
    return {"amd64": "x86_64", "x64": "x86_64", "arm64": "aarch64"}.get(raw, raw)


def library_name(os_name: str) -> str:
    return {"linux": f"lib{LIB}.so", "macos": f"lib{LIB}.dylib", "windows": f"{LIB}.dll"}[os_name]


def build(profile: str, target: str | None) -> None:
    cmd = ["cargo", "build", "-p", CRATE]
    if profile == "release":
        cmd.append("--release")
    if target:
        cmd += ["--target", target]
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, check=True)


def target_dir() -> Path:
    env = os.environ.get("CARGO_TARGET_DIR")
    return Path(env) if env else ROOT / "target"


def info_plist(version: str, kind: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>English</string>
  <key>CFBundleExecutable</key><string>{NAME}</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}.{kind}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>{NAME}</string>
  <key>CFBundleDisplayName</key><string>{NAME}</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleSignature</key><string>????</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"""


def fresh(path: Path) -> None:
    if path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()


def macos_bundle(lib: Path, dest: Path, version: str, kind: str) -> None:
    fresh(dest)
    (dest / "Contents" / "MacOS").mkdir(parents=True)
    shutil.copy2(lib, dest / "Contents" / "MacOS" / NAME)
    (dest / "Contents" / "Info.plist").write_text(info_plist(version, kind), encoding="utf-8")
    (dest / "Contents" / "PkgInfo").write_text("BNDL????", encoding="utf-8")


def bundle(lib: Path, out: Path, os_name: str, cpu: str, version: str) -> tuple[Path, Path]:
    out.mkdir(parents=True, exist_ok=True)
    clap = out / f"{NAME}.clap"
    vst3 = out / f"{NAME}.vst3"
    if os_name == "macos":
        macos_bundle(lib, clap, version, "clap")
        macos_bundle(lib, vst3, version, "vst3")
        return clap, vst3
    fresh(clap)
    shutil.copy2(lib, clap)
    fresh(vst3)
    if os_name == "windows":
        folder = {"x86_64": "x86_64-win", "aarch64": "arm64-win", "x86": "x86-win"}.get(cpu, f"{cpu}-win")
        binary = vst3 / "Contents" / folder / f"{NAME}.vst3"
    else:
        binary = vst3 / "Contents" / f"{cpu}-linux" / f"{NAME}.so"
    binary.parent.mkdir(parents=True)
    shutil.copy2(lib, binary)
    return clap, vst3


def plugin_folders(os_name: str) -> tuple[Path, Path]:
    """Per-user CLAP and VST3 folders hosts scan."""
    home = Path.home()
    if os_name == "macos":
        base = home / "Library" / "Audio" / "Plug-Ins"
        return base / "CLAP", base / "VST3"
    if os_name == "windows":
        local = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
        return local / "Programs" / "Common" / "CLAP", local / "Programs" / "Common" / "VST3"
    return home / ".clap", home / ".vst3"


def install(paths: tuple[Path, Path], os_name: str) -> None:
    for src, folder in zip(paths, plugin_folders(os_name)):
        folder.mkdir(parents=True, exist_ok=True)
        dest = folder / src.name
        fresh(dest)
        if src.is_dir():
            shutil.copytree(src, dest, symlinks=True)
        else:
            shutil.copy2(src, dest)
        print(f"installed {dest}")


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--debug", action="store_true", help="bundle a debug build")
    p.add_argument("--target", help="a target triple to build for (default: this machine)")
    p.add_argument("--out", type=Path, help="where to put the bundles (default: target/bundled)")
    p.add_argument("--no-build", action="store_true", help="bundle the library that's already built")
    p.add_argument("--install", action="store_true", help="copy the bundles into your plugin folders")
    args = p.parse_args()

    profile = "debug" if args.debug else "release"
    os_name = host_os(args.target)
    if not args.no_build:
        build(profile, args.target)
    built = target_dir() / (args.target or "") / profile / library_name(os_name)
    if not built.is_file():
        print(f"error: {built} not found; build it or drop --no-build", file=sys.stderr)
        return 1
    out = args.out or target_dir() / "bundled"
    paths = bundle(built, out, os_name, arch(args.target), workspace_version())
    for path in paths:
        print(f"bundled {path}")
    if args.install:
        install(paths, os_name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
