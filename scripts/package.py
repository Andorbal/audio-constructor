#!/usr/bin/env python3
"""Package a release build into this platform's archive and installer.

    python3 scripts/package.py --build                   # build, then package for this machine into dist/
    python3 scripts/package.py --target aarch64-apple-darwin --out dist
    python3 scripts/package.py --formats archive         # only some formats

Formats (all of a platform's by default):

- Linux:   `archive`, a .tar.gz with the programs, the plugins, the examples,
           and `install.sh` for an install in your home folder; and `deb`, a
           package that installs the same files system-wide.
- macOS:   `archive`, a .tar.gz with the programs and the plugins; and `pkg`,
           an installer that puts `Audio Constructor.app` in Applications,
           the plugins in /Library/Audio/Plug-Ins, and `audioc` in
           /usr/local/bin.
- Windows: `archive`, a .zip with the programs and the plugins; and `setup`,
           a setup program that installs the editor and `audioc`, a
           Start-menu entry, file types, the plugins into Common Files, and
           an uninstaller.

Signing happens when its credentials are in the environment, and is skipped
with a notice otherwise:

- macOS: MACOS_SIGNING_IDENTITY (a "Developer ID Application" identity in a
  keychain) signs the programs, the app, and the plugins with the hardened
  runtime; MACOS_INSTALLER_IDENTITY ("Developer ID Installer") signs the
  .pkg; with MACOS_NOTARY_APPLE_ID, MACOS_NOTARY_TEAM_ID, and
  MACOS_NOTARY_PASSWORD (an app-specific password) the .pkg is notarized
  and the ticket stapled to it.
- Windows: WINDOWS_CERTIFICATE (a .pfx file) and WINDOWS_CERTIFICATE_PASSWORD
  sign the programs, the plugin library, the setup program, and its
  uninstaller, with signtool from the Windows SDK and an RFC 3161 timestamp
  (WINDOWS_TIMESTAMP_URL, default DigiCert's public server).

Needs Python 3.8 or newer, and per format: tar or zip (built in),
dpkg-deb (deb), pkgbuild and productbuild (pkg; part of macOS), and
makensis from NSIS 3 (setup; it also runs on Linux and macOS; found on the
PATH, in NSIS's install folder, or at MAKENSIS).
"""

from __future__ import annotations

import argparse
import hashlib
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import zipfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NAME = "Audio Constructor"
SLUG = "audio-constructor"
BUNDLE_ID = "io.github.andorbal.audio-constructor"
PROGRAMS = ["audio-constructor", "audioc"]
PLUGIN_BUNDLES = [f"{NAME}.clap", f"{NAME}.vst3"]
ICON_SIZES = [16, 32, 48, 64, 128, 256, 512]
DOCS = ["README.md", "LICENSE", "CHANGELOG.md"]
FORMATS = {"linux": ["archive", "deb"], "macos": ["archive", "pkg"], "windows": ["archive", "setup"]}
DEB_DEPENDS = "libasound2t64 | libasound2, libudev1, libgl1, libxkbcommon0"
DEB_RECOMMENDS = "libjack-jackd2-0 | libjack0"
DEFAULT_TIMESTAMP_URL = "http://timestamp.digicert.com"


class PackageError(Exception):
    pass


# --- What we're packaging -----------------------------------------------------


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]+)"', text, re.M | re.S)
    if not m:
        raise PackageError("no version under [workspace.package] in Cargo.toml")
    return m.group(1)


def host_target() -> str:
    out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        if line.startswith("host:"):
            return line.split(":", 1)[1].strip()
    raise PackageError("rustc -vV printed no host")


def os_of(target: str) -> str:
    if "apple" in target:
        return "macos"
    if "windows" in target:
        return "windows"
    return "linux"


def cpu_of(target: str) -> str:
    return target.split("-")[0]


def target_dir() -> Path:
    env = os.environ.get("CARGO_TARGET_DIR")
    return Path(env) if env else ROOT / "target"


def default_bin_dir(target: str) -> Path:
    """Where cargo put the release programs: `target/<triple>/release` after
    a `--target` build, `target/release` after a plain one on this machine."""
    explicit = target_dir() / target / "release"
    plain = target_dir() / "release"
    if not (explicit / "audioc").exists() and not (explicit / "audioc.exe").exists() and target == host_target():
        return plain
    return explicit


@dataclass
class Build:
    version: str
    target: str
    os: str
    cpu: str
    bin_dir: Path
    plugins_dir: Path
    out: Path
    work: Path

    @property
    def exe(self) -> str:
        return ".exe" if self.os == "windows" else ""

    @property
    def stem(self) -> str:
        """The archive's and installers' base name."""
        return f"{SLUG}-v{self.version}-{self.target}"

    def program(self, name: str) -> Path:
        return self.bin_dir / f"{name}{self.exe}"


def run(cmd: list, **kw) -> None:
    print("+", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run([str(c) for c in cmd], check=True, **kw)


def fresh(path: Path) -> Path:
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    elif path.exists() or path.is_symlink():
        path.unlink()
    return path


def copy(src: Path, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    if src.is_dir():
        shutil.copytree(src, dest, symlinks=True)
    else:
        shutil.copy2(src, dest)


def dir_size_kb(path: Path) -> int:
    total = sum(f.stat().st_size for f in path.rglob("*") if f.is_file() and not f.is_symlink())
    return (total + 1023) // 1024


def check_inputs(b: Build) -> None:
    missing = [p for p in [b.program(n) for n in PROGRAMS] + [b.plugins_dir / n for n in PLUGIN_BUNDLES] if not p.exists()]
    if missing:
        listed = "\n  ".join(str(p) for p in missing)
        raise PackageError(f"missing build outputs (build with --build, or point --bin-dir/--plugins at them):\n  {listed}")


# --- Building -------------------------------------------------------------------


def build_all(target: str) -> None:
    env = dict(os.environ)
    # Symbols stay, so backtraces still name functions; the line tables would
    # make the Linux editor ~170 MB (release.yml's note).
    env.setdefault("CARGO_PROFILE_RELEASE_STRIP", "debuginfo")
    run(["cargo", "build", "--release", "--locked", "--target", target, "-p", "ac-cli", "-p", "ac-app"], cwd=ROOT, env=env)
    run([sys.executable, ROOT / "scripts" / "bundle_plugin.py", "--target", target], cwd=ROOT, env=env)


# --- Signing --------------------------------------------------------------------


def macos_identity() -> str | None:
    return os.environ.get("MACOS_SIGNING_IDENTITY") or None


def codesign(path: Path, entitlements: bool = False) -> None:
    identity = macos_identity()
    if not identity:
        return
    cmd = ["codesign", "--force", "--timestamp", "--options", "runtime", "--sign", identity]
    if entitlements:
        cmd += ["--entitlements", ROOT / "packaging" / "macos" / "entitlements.plist"]
    run(cmd + [path])
    run(["codesign", "--verify", "--strict", "--verbose=2", path])


def find_signtool() -> str | None:
    found = shutil.which("signtool")
    if found:
        return found
    kits = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits" / "10" / "bin"
    candidates = sorted(kits.glob("*/x64/signtool.exe"), key=lambda p: [int(x) if x.isdigit() else 0 for x in p.parent.parent.name.split(".")])
    return str(candidates[-1]) if candidates else None


def windows_signing() -> bool:
    return bool(os.environ.get("WINDOWS_CERTIFICATE") and os.environ.get("WINDOWS_CERTIFICATE_PASSWORD"))


def signtool(path: Path) -> None:
    tool = find_signtool()
    if not tool:
        raise PackageError("WINDOWS_CERTIFICATE is set but signtool wasn't found (install the Windows SDK)")
    url = os.environ.get("WINDOWS_TIMESTAMP_URL", DEFAULT_TIMESTAMP_URL)
    cert = os.environ["WINDOWS_CERTIFICATE"]
    password = os.environ["WINDOWS_CERTIFICATE_PASSWORD"]
    # The password goes straight to signtool, never through a shell or a log.
    print(f"+ signtool sign /f <certificate> /fd sha256 /tr {url} /td sha256 {path}", flush=True)
    subprocess.run([tool, "sign", "/f", cert, "/p", password, "/fd", "sha256", "/tr", url, "/td", "sha256", str(path)], check=True)
    run([tool, "verify", "/pa", path])


def sign_programs(b: Build, folder: Path) -> list[str]:
    """Sign the staged programs and plugins in `folder`; what was signed, for the summary."""
    if b.os == "macos" and macos_identity():
        for name in PROGRAMS:
            codesign(folder / name, entitlements=True)
        for bundle in PLUGIN_BUNDLES:
            codesign(folder / "plugins" / bundle)
        return ["programs and plugins (Developer ID, hardened runtime)"]
    if b.os == "windows" and windows_signing():
        for name in PROGRAMS:
            signtool(folder / f"{name}.exe")
        signtool(folder / "plugins" / f"{NAME}.clap")
        for inner in (folder / "plugins" / f"{NAME}.vst3").rglob("*.vst3"):
            if inner.is_file():
                signtool(inner)
        return ["programs and plugins (Authenticode)"]
    return []


# --- Staging: what every package holds -----------------------------------------


def stage(b: Build) -> Path:
    """The programs, plugins, docs, and examples, laid out as the archive has them."""
    folder = fresh(b.work / b.stem)
    folder.mkdir(parents=True)
    for name in PROGRAMS:
        copy(b.program(name), folder / f"{name}{b.exe}")
    for bundle in PLUGIN_BUNDLES:
        copy(b.plugins_dir / bundle, folder / "plugins" / bundle)
    for doc in DOCS:
        copy(ROOT / doc, folder / doc)
    copy(ROOT / "examples", folder / "examples")
    (folder / "VERSION").write_text(b.version + "\n", encoding="utf-8")
    if b.os == "linux":
        copy(ROOT / "packaging" / "linux" / "install.sh", folder / "install.sh")
        (folder / "install.sh").chmod(0o755)
        share = folder / "share"
        copy(ROOT / "packaging" / "linux" / f"{SLUG}.desktop", share / f"{SLUG}.desktop")
        copy(ROOT / "packaging" / "linux" / f"{SLUG}.xml", share / f"{SLUG}.xml")
        for size in ICON_SIZES:
            copy(ROOT / "packaging" / "icons" / f"icon-{size}.png", share / "icons" / f"icon-{size}.png")
    return folder


def make_archive(b: Build, folder: Path) -> Path:
    if b.os == "windows":
        dest = fresh(b.out / f"{b.stem}.zip")
        with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for f in sorted(folder.rglob("*")):
                if f.is_file():
                    z.write(f, f.relative_to(folder.parent).as_posix())
        return dest
    dest = fresh(b.out / f"{b.stem}.tar.gz")

    def owned_by_root(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        return info

    with tarfile.open(dest, "w:gz", compresslevel=9) as t:
        t.add(folder, arcname=folder.name, filter=owned_by_root)
    return dest


# --- Linux: .deb ------------------------------------------------------------------


def deb_arch(cpu: str) -> str:
    return {"x86_64": "amd64", "aarch64": "arm64", "i686": "i386", "armv7": "armhf"}.get(cpu, cpu)


def make_deb(b: Build, folder: Path) -> Path:
    if not shutil.which("dpkg-deb"):
        raise PackageError("dpkg-deb not found (it comes with dpkg)")
    root = fresh(b.work / "deb")
    usr = root / "usr"
    for name in PROGRAMS:
        copy(folder / name, usr / "bin" / name)
    copy(folder / "examples", usr / "share" / SLUG / "examples")
    copy(folder / "share" / f"{SLUG}.desktop", usr / "share" / "applications" / f"{SLUG}.desktop")
    copy(folder / "share" / f"{SLUG}.xml", usr / "share" / "mime" / "packages" / f"{SLUG}.xml")
    for size in ICON_SIZES:
        copy(folder / "share" / "icons" / f"icon-{size}.png", usr / "share" / "icons" / "hicolor" / f"{size}x{size}" / "apps" / f"{SLUG}.png")
    # The system-wide folders hosts scan (the CLAP and VST3 conventions).
    copy(folder / "plugins" / f"{NAME}.clap", usr / "lib" / "clap" / f"{NAME}.clap")
    copy(folder / "plugins" / f"{NAME}.vst3", usr / "lib" / "vst3" / f"{NAME}.vst3")
    doc = usr / "share" / "doc" / SLUG
    copy(ROOT / "LICENSE", doc / "copyright")
    copy(ROOT / "README.md", doc / "README.md")
    for f in root.rglob("*"):
        f.chmod(0o755 if f.is_dir() or f.parent.name == "bin" or f.suffix in (".clap", ".so") else 0o644)
    control = root / "DEBIAN" / "control"
    control.parent.mkdir(parents=True)
    control.parent.chmod(0o755)
    control.write_text(
        "\n".join(
            [
                f"Package: {SLUG}",
                f"Version: {b.version}",
                f"Architecture: {deb_arch(b.cpu)}",
                "Maintainer: Audio Constructor contributors <noreply@github.com>",
                f"Installed-Size: {dir_size_kb(usr)}",
                f"Depends: {DEB_DEPENDS}",
                f"Recommends: {DEB_RECOMMENDS}",
                "Section: sound",
                "Priority: optional",
                "Homepage: https://github.com/Andorbal/audio-constructor",
                "Description: node-based audio construction suite",
                " Build instruments, effects, and generative pieces by wiring nodes",
                " together, and play them live, render them to files, or run them as",
                " CLAP and VST3 plugins. Includes the editor, the audioc command-line",
                " tool, the plugins, and the examples.",
                "",
            ]
        ),
        encoding="utf-8",
    )
    control.chmod(0o644)
    dest = fresh(b.out / f"{SLUG}_{b.version}_{deb_arch(b.cpu)}.deb")
    run(["dpkg-deb", "--root-owner-group", "-Zxz", "--build", root, dest])
    return dest


# --- macOS: the app and the .pkg ---------------------------------------------------


def app_plist(version: str) -> dict:
    return {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": NAME,
        "CFBundleExecutable": "audio-constructor",
        "CFBundleIconFile": "icon",
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": NAME,
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSApplicationCategoryType": "public.app-category.music",
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
        "NSMicrophoneUsageDescription": "Audio In nodes play and record what your microphone or audio interface hears.",
        "NSHumanReadableCopyright": "MIT License",
    }


def make_app(b: Build, folder: Path) -> Path:
    app = fresh(b.work / "app" / f"{NAME}.app")
    contents = app / "Contents"
    for name in PROGRAMS:
        copy(folder / name, contents / "MacOS" / name)
    copy(ROOT / "packaging" / "icons" / "icon.icns", contents / "Resources" / "icon.icns")
    copy(folder / "examples", contents / "Resources" / "examples")
    with open(contents / "Info.plist", "wb") as f:
        plistlib.dump(app_plist(b.version), f)
    (contents / "PkgInfo").write_text("APPL????", encoding="utf-8")
    # Inside out: the second program, then the bundle (whose main
    # executable is signed with it).
    codesign(contents / "MacOS" / "audioc", entitlements=True)
    codesign(app, entitlements=True)
    return app


def component_plist(path: Path, bundles: list[str]) -> Path:
    # Without this, the installer "relocates" an update to wherever a copy of
    # the bundle was found on the disk instead of where it belongs.
    with open(path, "wb") as f:
        plistlib.dump(
            [
                {
                    "RootRelativeBundlePath": rel,
                    "BundleIsRelocatable": False,
                    "BundleIsVersionChecked": False,
                    "BundleHasStrictIdentifier": False,
                    "BundleOverwriteAction": "upgrade",
                }
                for rel in bundles
            ],
            f,
        )
    return path


POSTINSTALL = """#!/bin/sh
# audioc on the PATH of new terminals, pointing into the app.
mkdir -p /usr/local/bin
ln -sf "/Applications/Audio Constructor.app/Contents/MacOS/audioc" /usr/local/bin/audioc
exit 0
"""


def distribution_xml(b: Build) -> str:
    arch = {"aarch64": "arm64"}.get(b.cpu, b.cpu)
    return f"""<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
  <title>{NAME} {b.version}</title>
  <license file="LICENSE.txt"/>
  <options customize="allow" require-scripts="false" hostArchitectures="{arch}"/>
  <domains enable_localSystem="true"/>
  <allowed-os-versions><os-version min="11.0"/></allowed-os-versions>
  <choices-outline>
    <line choice="app"/>
    <line choice="clap"/>
    <line choice="vst3"/>
  </choices-outline>
  <choice id="app" title="{NAME}" description="The editor in Applications, and audioc in /usr/local/bin." enabled="false">
    <pkg-ref id="{BUNDLE_ID}.app"/>
  </choice>
  <choice id="clap" title="CLAP plugins" description="Audio Constructor Effect and Instrument, in /Library/Audio/Plug-Ins/CLAP.">
    <pkg-ref id="{BUNDLE_ID}.clap"/>
  </choice>
  <choice id="vst3" title="VST3 plugins" description="Audio Constructor Effect and Instrument, in /Library/Audio/Plug-Ins/VST3.">
    <pkg-ref id="{BUNDLE_ID}.vst3"/>
  </choice>
  <pkg-ref id="{BUNDLE_ID}.app" version="{b.version}" onConclusion="none">app.pkg</pkg-ref>
  <pkg-ref id="{BUNDLE_ID}.clap" version="{b.version}" onConclusion="none">clap.pkg</pkg-ref>
  <pkg-ref id="{BUNDLE_ID}.vst3" version="{b.version}" onConclusion="none">vst3.pkg</pkg-ref>
</installer-gui-script>
"""


def make_pkg(b: Build, folder: Path) -> Path:
    for tool in ("pkgbuild", "productbuild"):
        if not shutil.which(tool):
            raise PackageError(f"{tool} not found (it comes with macOS)")
    app = make_app(b, folder)
    work = fresh(b.work / "pkg")
    parts = work / "parts"
    parts.mkdir(parents=True)

    roots = {"app": work / "root-app", "clap": work / "root-clap", "vst3": work / "root-vst3"}
    copy(app, roots["app"] / app.name)
    copy(folder / "plugins" / f"{NAME}.clap", roots["clap"] / f"{NAME}.clap")
    copy(folder / "plugins" / f"{NAME}.vst3", roots["vst3"] / f"{NAME}.vst3")
    scripts = work / "scripts"
    scripts.mkdir()
    (scripts / "postinstall").write_text(POSTINSTALL, encoding="utf-8")
    (scripts / "postinstall").chmod(0o755)

    locations = {"app": "/Applications", "clap": "/Library/Audio/Plug-Ins/CLAP", "vst3": "/Library/Audio/Plug-Ins/VST3"}
    for part, root in roots.items():
        bundle = next(root.iterdir()).name
        cmd = [
            "pkgbuild",
            "--root", root,
            "--component-plist", component_plist(work / f"{part}.plist", [bundle]),
            "--identifier", f"{BUNDLE_ID}.{part}",
            "--version", b.version,
            "--install-location", locations[part],
        ]
        if part == "app":
            cmd += ["--scripts", scripts]
        run(cmd + [parts / f"{part}.pkg"])

    resources = work / "resources"
    resources.mkdir()
    copy(ROOT / "LICENSE", resources / "LICENSE.txt")
    dist = work / "distribution.xml"
    dist.write_text(distribution_xml(b), encoding="utf-8")
    dest = fresh(b.out / f"{b.stem}.pkg")
    cmd = ["productbuild", "--distribution", dist, "--package-path", parts, "--resources", resources]
    installer_identity = os.environ.get("MACOS_INSTALLER_IDENTITY")
    if installer_identity:
        cmd += ["--sign", installer_identity, "--timestamp"]
    run(cmd + [dest])
    if installer_identity:
        notarize(dest)
    return dest


def notarize(pkg: Path) -> None:
    creds = [os.environ.get(k) for k in ("MACOS_NOTARY_APPLE_ID", "MACOS_NOTARY_TEAM_ID", "MACOS_NOTARY_PASSWORD")]
    if not all(creds):
        print("note: the .pkg is signed but not notarized (MACOS_NOTARY_* aren't all set)", flush=True)
        return
    apple_id, team, password = creds
    print(f"+ xcrun notarytool submit {pkg} --apple-id <id> --team-id {team} --password <password> --wait", flush=True)
    subprocess.run(
        ["xcrun", "notarytool", "submit", str(pkg), "--apple-id", apple_id, "--team-id", team, "--password", password, "--wait"],
        check=True,
    )
    run(["xcrun", "stapler", "staple", pkg])
    run(["xcrun", "stapler", "validate", pkg])


# --- Windows: the setup program ---------------------------------------------------


def find_makensis() -> str | None:
    """makensis on the PATH, or where NSIS's Windows installer puts it."""
    found = os.environ.get("MAKENSIS") or shutil.which("makensis")
    if found:
        return found
    for var in ("ProgramFiles(x86)", "ProgramFiles"):
        base = os.environ.get(var)
        if base and (Path(base) / "NSIS" / "makensis.exe").is_file():
            return str(Path(base) / "NSIS" / "makensis.exe")
    return None


def make_setup(b: Build, folder: Path) -> Path:
    makensis = find_makensis()
    if not makensis:
        raise PackageError("makensis not found (install NSIS 3, or set MAKENSIS to it)")
    # NSIS takes the files from a staging folder named as the script expects.
    nsis = fresh(b.work / "nsis")
    for name in PROGRAMS:
        copy(folder / f"{name}{b.exe}", nsis / f"{name}.exe")
    for doc in ("README.md", "CHANGELOG.md"):
        copy(folder / doc, nsis / doc)
    copy(ROOT / "LICENSE", nsis / "LICENSE.txt")
    copy(ROOT / "packaging" / "icons" / "icon.ico", nsis / "icon.ico")
    copy(folder / "examples", nsis / "examples")
    copy(folder / "plugins", nsis / "plugins")
    dest = fresh(b.out / f"{b.stem}-setup.exe")
    # makensis takes `-D` on every platform; `/D` would be a path on Unix.
    cmd = [
        makensis, "-V3", "-NOCD",
        f"-DVERSION={b.version}",
        f"-DSTAGE={nsis}",
        f"-DOUTFILE={dest}",
        f"-DSIZE_KB={dir_size_kb(nsis)}",
    ]
    if windows_signing():
        cmd += [f"-DSIGN_PY={sys.executable}", f"-DSIGN_SCRIPT={Path(__file__).resolve()}"]
    run(cmd + [ROOT / "packaging" / "windows" / "installer.nsi"])
    return dest


# --- Main -------------------------------------------------------------------------


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def package(b: Build, formats: list[str]) -> list[Path]:
    check_inputs(b)
    b.out.mkdir(parents=True, exist_ok=True)
    folder = stage(b)
    signed = sign_programs(b, folder)
    made = []
    for fmt in formats:
        if fmt == "archive":
            made.append(make_archive(b, folder))
        elif fmt == "deb":
            made.append(make_deb(b, folder))
        elif fmt == "pkg":
            made.append(make_pkg(b, folder))
        elif fmt == "setup":
            made.append(make_setup(b, folder))
        else:
            raise PackageError(f"unknown format {fmt!r}")
    if b.os != "linux" and not signed:
        needs = "MACOS_SIGNING_IDENTITY" if b.os == "macos" else "WINDOWS_CERTIFICATE and WINDOWS_CERTIFICATE_PASSWORD"
        print(f"note: nothing was signed ({needs} not set)", flush=True)
    for path in made:
        print(f"made {path} ({path.stat().st_size // 1024} KiB, sha256 {sha256(path)[:16]}…)", flush=True)
    return made


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--target", help="the target triple (default: this machine's)")
    p.add_argument("--build", action="store_true", help="build the programs and plugins in release first")
    p.add_argument("--bin-dir", type=Path, help="where the release programs are (default: target/<target>/release, or target/release)")
    p.add_argument("--plugins", type=Path, help="where the plugin bundles are (default: target/bundled)")
    p.add_argument("--out", type=Path, default=ROOT / "dist", help="where the packages go (default: dist/)")
    p.add_argument("--formats", help="comma-separated: archive, deb, pkg, setup (default: all of this platform's)")
    p.add_argument("--version", help="the version to package as (default: the workspace's)")
    p.add_argument("--sign-file", type=Path, help=argparse.SUPPRESS)
    args = p.parse_args()

    try:
        if args.sign_file:
            # NSIS calls back here to sign the installer and uninstaller.
            signtool(args.sign_file)
            return 0
        target = args.target or host_target()
        os_name = os_of(target)
        if args.build:
            build_all(target)
        formats = args.formats.split(",") if args.formats else FORMATS[os_name]
        b = Build(
            version=args.version or workspace_version(),
            target=target,
            os=os_name,
            cpu=cpu_of(target),
            bin_dir=args.bin_dir or default_bin_dir(target),
            plugins_dir=args.plugins or target_dir() / "bundled",
            out=args.out.resolve(),
            work=(args.out / ".work").resolve(),
        )
        package(b, formats)
    except (PackageError, subprocess.CalledProcessError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
