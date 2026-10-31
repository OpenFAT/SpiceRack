"""Bump the package version in Cargo.toml, pyproject.toml and Cargo.lock.

    python3 tools/bump_version.py patch|minor|major|X.Y.Z

Prints ``version=X.Y.Z`` (for ``$GITHUB_OUTPUT``). Refuses manifests that
disagree, and a version that does not move forward. Release workflow:
.github/workflows/publish.yaml.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFESTS = ("Cargo.toml", "pyproject.toml")
SEMVER = re.compile(r"(\d+)\.(\d+)\.(\d+)")
VERSION_LINE = re.compile(r'(?m)^version = "([^"]+)"$')


def current(root: Path) -> str:
    found = {f: VERSION_LINE.search((root / f).read_text()).group(1) for f in MANIFESTS}
    if len(set(found.values())) != 1:
        sys.exit(f"manifests disagree: {found}")
    return found[MANIFESTS[0]]


def bumped(old: str, how: str) -> str:
    m = SEMVER.fullmatch(old)
    if not m:
        sys.exit(f"current version {old!r} is not X.Y.Z")
    major, minor, patch = map(int, m.groups())
    new = {"major": f"{major + 1}.0.0", "minor": f"{major}.{minor + 1}.0",
           "patch": f"{major}.{minor}.{patch + 1}"}.get(how, how)
    n = SEMVER.fullmatch(new)
    if not n or tuple(map(int, n.groups())) <= (major, minor, patch):
        sys.exit(f"{how!r} is not patch, minor, major or an X.Y.Z above {old}")
    return new


def bump(root: Path, how: str) -> str:
    old = current(root)
    new = bumped(old, how)
    for f in MANIFESTS:
        p = root / f
        p.write_text(VERSION_LINE.sub(f'version = "{new}"', p.read_text(), count=1))
    lock = root / "Cargo.lock"
    entry = f'name = "spicerack"\nversion = "{old}"\n'
    text = lock.read_text()
    if entry not in text:
        sys.exit("Cargo.lock has no spicerack entry at the current version")
    lock.write_text(text.replace(entry, f'name = "spicerack"\nversion = "{new}"\n', 1))
    return new


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    print(f"version={bump(ROOT, sys.argv[1])}")
