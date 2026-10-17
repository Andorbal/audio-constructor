#!/usr/bin/env python3
"""Version and changelog helper for merge requests and releases.

Every crate shares the workspace version (`[workspace.package] version` in
Cargo.toml). Each merge request raises it and adds a section for it to
CHANGELOG.md; merging to `main` releases that version as `vX.Y.Z` (see
.github/workflows/release.yml and the "Versions, changelog, and releases"
section of CLAUDE.md).

    python3 scripts/release.py bump minor     # or patch, major, X.Y.Z
    python3 scripts/release.py check          # what CI checks on a merge request
    python3 scripts/release.py version        # print the workspace version
    python3 scripts/release.py notes [X.Y.Z]  # print a version's changelog section

Needs Python 3.11 or newer and git; nothing else.
"""

from __future__ import annotations

import argparse
import datetime
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
CHANGELOG = ROOT / "CHANGELOG.md"
DEFAULT_BASE = "origin/main"

SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
HEADING = re.compile(r"^## \[(?P<version>[^\]]+)\](?: - (?P<date>\S+))?\s*$")
LINK_REF = re.compile(r"^\[(?P<version>[^\]]+)\]: (?P<url>\S+)\s*$")


class ReleaseError(Exception):
    pass


def parse_version(text: str) -> tuple[int, int, int]:
    m = SEMVER.match(text)
    if not m:
        raise ReleaseError(f"`{text}` is not a version of the form X.Y.Z")
    return int(m[1]), int(m[2]), int(m[3])


def next_version(current: str, spec: str) -> str:
    major, minor, patch = parse_version(current)
    if spec == "major":
        return f"{major + 1}.0.0"
    if spec == "minor":
        return f"{major}.{minor + 1}.0"
    if spec == "patch":
        return f"{major}.{minor}.{patch + 1}"
    if parse_version(spec) <= parse_version(current):
        raise ReleaseError(f"{spec} is not newer than {current}")
    return spec


# --- Cargo.toml and Cargo.lock ----------------------------------------------


def workspace_version(cargo_toml: str) -> str:
    try:
        return tomllib.loads(cargo_toml)["workspace"]["package"]["version"]
    except (tomllib.TOMLDecodeError, KeyError) as e:
        raise ReleaseError(f"no [workspace.package] version in Cargo.toml ({e})") from e


def set_workspace_version(cargo_toml: str, version: str) -> str:
    # Edit the one line in place so comments and formatting survive.
    lines = cargo_toml.splitlines(keepends=True)
    in_section = False
    for i, line in enumerate(lines):
        if line.startswith("["):
            in_section = line.strip() == "[workspace.package]"
        elif in_section and re.match(r"^version\s*=", line):
            lines[i] = f'version = "{version}"\n'
            return "".join(lines)
    raise ReleaseError("no version line in Cargo.toml's [workspace.package]")


def crates_on_workspace_version() -> set[str]:
    """Names of the member crates whose version is `version.workspace = true`."""
    members = tomllib.loads(CARGO_TOML.read_text())["workspace"]["members"]
    names = set()
    for pattern in members:
        for manifest in sorted(ROOT.glob(f"{pattern}/Cargo.toml")):
            package = tomllib.loads(manifest.read_text()).get("package", {})
            if package.get("version") == {"workspace": True}:
                names.add(package["name"])
    return names


def set_lock_versions(cargo_lock: str, names: set[str], version: str) -> str:
    # Packages in Cargo.lock are blank-line-separated blocks that start with
    # name and version; local crates are the ones without a `source`.
    def fix(block: str) -> str:
        m = re.match(r'\[\[package\]\]\nname = "([^"]+)"\nversion = "[^"]+"(?:\n|$)', block)
        if m and m[1] in names and "\nsource = " not in block:
            return re.sub(r'^version = "[^"]+"$', f'version = "{version}"', block, count=1, flags=re.M)
        return block

    return "\n\n".join(fix(block) for block in cargo_lock.split("\n\n"))


def stale_lock_entries(cargo_lock: str, names: set[str], version: str) -> list[str]:
    packages = tomllib.loads(cargo_lock).get("package", [])
    return sorted(
        f"{p['name']} {p['version']}"
        for p in packages
        if p["name"] in names and "source" not in p and p["version"] != version
    )


# --- CHANGELOG.md -------------------------------------------------------------


@dataclass
class Section:
    version: str
    date: str | None
    heading: int  # line index of the `## [X.Y.Z] - date` heading
    end: int  # line index one past the section's last line

    def body(self, lines: list[str]) -> str:
        return "".join(lines[self.heading + 1 : self.end]).strip()


def parse_changelog(text: str) -> tuple[list[str], list[Section], dict[str, int]]:
    """The file's lines, its version sections (newest first), and the line of
    each `[X.Y.Z]: url` link reference."""
    lines = text.splitlines(keepends=True)
    sections: list[Section] = []
    links: dict[str, int] = {}
    for i, line in enumerate(lines):
        if m := HEADING.match(line):
            if sections and sections[-1].end == len(lines):
                sections[-1].end = i
            sections.append(Section(m["version"], m["date"], i, len(lines)))
        elif m := LINK_REF.match(line):
            if sections and sections[-1].end == len(lines):
                sections[-1].end = i
            links[m["version"]] = i
        elif line.startswith("## ") and sections and sections[-1].end == len(lines):
            sections[-1].end = i
    return lines, sections, links


def link_for(repo: str, version: str, previous: str | None) -> str:
    if previous is None:
        return f"[{version}]: {repo}/releases/tag/v{version}\n"
    return f"[{version}]: {repo}/compare/v{previous}...v{version}\n"


def write_section(changelog: str, repo: str, version: str, date: str, replace: str | None) -> str:
    """Give `version` the newest section, dated `date`. With `replace`, that
    section is renamed (a branch changing its own bump); otherwise a new,
    empty section goes above the newest one."""
    lines, sections, links = parse_changelog(changelog)
    heading = f"## [{version}] - {date}\n"
    if replace is not None:
        target = next(s for s in sections if s.version == replace)
        previous = next((s.version for s in sections if s.heading > target.heading), None)
        lines[target.heading] = heading
        if replace in links:
            lines[links[replace]] = link_for(repo, version, previous)
            return "".join(lines)
    else:
        previous = sections[0].version if sections else None
        at = sections[0].heading if sections else len(lines)
        lines[at:at] = [heading, "\n"]
        links = {v: i + 2 if i >= at else i for v, i in links.items()}
    # Link references are kept newest first at the end of the file.
    link = link_for(repo, version, previous)
    if links:
        at = min(links.values())
        lines[at:at] = [link]
    else:
        if lines and lines[-1].strip():
            lines.append("\n")
        lines.append(link)
    return "".join(lines)


# --- git ----------------------------------------------------------------------


def git_show(rev: str, path: str) -> str | None:
    result = subprocess.run(
        ["git", "show", f"{rev}:{path}"], cwd=ROOT, capture_output=True, text=True
    )
    return result.stdout if result.returncode == 0 else None


def base_version(base: str) -> str:
    text = git_show(base, "Cargo.toml")
    if text is None:
        raise ReleaseError(
            f"can't read Cargo.toml at `{base}`; fetch it first (git fetch origin main) or pass --base"
        )
    return workspace_version(text)


# --- commands -----------------------------------------------------------------


def cmd_version(_args: argparse.Namespace) -> int:
    print(workspace_version(CARGO_TOML.read_text()))
    return 0


def cmd_notes(args: argparse.Namespace) -> int:
    version = args.version or workspace_version(CARGO_TOML.read_text())
    lines, sections, _ = parse_changelog(CHANGELOG.read_text())
    section = next((s for s in sections if s.version == version), None)
    if section is None or not section.body(lines):
        raise ReleaseError(f"CHANGELOG.md has no entries for {version}")
    print(section.body(lines))
    return 0


def cmd_bump(args: argparse.Namespace) -> int:
    cargo_toml = CARGO_TOML.read_text()
    current = workspace_version(cargo_toml)
    base = base_version(args.base)
    # Bump from what the base branch has, so changing your mind (patch, then
    # minor) or refreshing the date renames this branch's section instead of
    # stacking a second one on it.
    already_bumped = parse_version(current) > parse_version(base)
    version = next_version(base if already_bumped else current, args.part)
    date = args.date or datetime.date.today().isoformat()
    try:
        datetime.date.fromisoformat(date)
    except ValueError as e:
        raise ReleaseError(f"--date: {e}") from e

    CARGO_TOML.write_text(set_workspace_version(cargo_toml, version))
    CARGO_LOCK.write_text(set_lock_versions(CARGO_LOCK.read_text(), crates_on_workspace_version(), version))

    repo = tomllib.loads(cargo_toml)["workspace"]["package"]["repository"].rstrip("/")
    changelog = CHANGELOG.read_text()
    _, sections, _ = parse_changelog(changelog)
    replace = current if already_bumped and any(s.version == current for s in sections) else None
    CHANGELOG.write_text(write_section(changelog, repo, version, date, replace))

    print(f"{current} -> {version} ({args.base} has {base})")
    if replace is None:
        print(f"Add entries under `## [{version}] - {date}` in CHANGELOG.md, in the commits that make the changes.")
    else:
        print(f"CHANGELOG.md: the {current} section is now `## [{version}] - {date}`.")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    problems: list[str] = []
    head = workspace_version(CARGO_TOML.read_text())
    try:
        parse_version(head)
    except ReleaseError as e:
        problems.append(f"Cargo.toml: {e}")

    stale = stale_lock_entries(CARGO_LOCK.read_text(), crates_on_workspace_version(), head)
    if stale:
        problems.append(
            f"Cargo.lock still has {', '.join(stale)}; any cargo command (`cargo check`) refreshes it"
        )

    base = base_version(args.base)
    first_release = git_show(args.base, "CHANGELOG.md") is None
    if SEMVER.match(head) and SEMVER.match(base) and parse_version(head) <= parse_version(base):
        # The merge request that adds the changelog releases the version that
        # was already in Cargo.toml; every later one must raise it.
        if not (first_release and head == base):
            problems.append(
                f"the version is {head} and `{args.base}` already has {base}; every merge request "
                f"releases a new version: run `python3 scripts/release.py bump patch` (or minor)"
            )

    if not CHANGELOG.is_file():
        problems.append("CHANGELOG.md is missing")
    else:
        lines, sections, links = parse_changelog(CHANGELOG.read_text())
        seen = [s.version for s in sections]
        duplicates = sorted({v for v in seen if seen.count(v) > 1})
        if duplicates:
            problems.append(f"CHANGELOG.md has more than one section for {', '.join(duplicates)}")
        if not sections or sections[0].version != head:
            newest = f"[{sections[0].version}]" if sections else "missing"
            problems.append(
                f"CHANGELOG.md's newest section is {newest}; it must be `## [{head}] - YYYY-MM-DD`"
            )
        else:
            top = sections[0]
            try:
                datetime.date.fromisoformat(top.date or "")
            except ValueError:
                problems.append(f"CHANGELOG.md: `{lines[top.heading].strip()}` needs a YYYY-MM-DD date")
            entries = [l for l in lines[top.heading + 1 : top.end] if re.match(r"^\s*[-*] \S", l)]
            if not entries:
                problems.append(f"CHANGELOG.md: the {head} section has no entries")
            if head not in links:
                problems.append(f"CHANGELOG.md has no `[{head}]: ...` link reference at the end")

    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
            if os.environ.get("GITHUB_ACTIONS") == "true":
                print(f"::error::{problem}")
        return 1
    if first_release and head == base:
        print(f"ok: first release, {head}, and CHANGELOG.md has its section")
    else:
        print(f"ok: {head} is newer than {args.base} ({base}) and CHANGELOG.md has its section")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)

    commands.add_parser("version", help="print the workspace version").set_defaults(run=cmd_version)

    notes = commands.add_parser("notes", help="print a version's changelog section")
    notes.add_argument("version", nargs="?", help="default: the workspace version")
    notes.set_defaults(run=cmd_notes)

    bump = commands.add_parser("bump", help="raise the version and open its changelog section")
    bump.add_argument("part", help="major, minor, patch, or an explicit X.Y.Z")
    bump.add_argument("--base", default=DEFAULT_BASE, help=f"the branch this merges into (default {DEFAULT_BASE})")
    bump.add_argument("--date", help="section date, YYYY-MM-DD (default today)")
    bump.set_defaults(run=cmd_bump)

    check = commands.add_parser("check", help="check the version bump and changelog against the base branch")
    check.add_argument("--base", default=DEFAULT_BASE, help=f"the branch this merges into (default {DEFAULT_BASE})")
    check.set_defaults(run=cmd_check)

    args = parser.parse_args()
    try:
        return args.run(args)
    except ReleaseError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
