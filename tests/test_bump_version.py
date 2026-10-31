"""tools/bump_version.py on a copy of the manifests: each bump kind, and the refusals."""
import importlib.util
import shutil
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("bump_version", ROOT / "tools" / "bump_version.py")
bv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bv)


@pytest.fixture
def tree(tmp_path):
    for f in ("Cargo.toml", "pyproject.toml", "Cargo.lock"):
        shutil.copy(ROOT / f, tmp_path / f)
    for f in ("Cargo.toml", "pyproject.toml"):  # pin a known start
        p = tmp_path / f
        p.write_text(bv.VERSION_LINE.sub('version = "1.4.9"', p.read_text(), count=1))
    lock = tmp_path / "Cargo.lock"
    lock.write_text(lock.read_text().replace(f'name = "spicerack"\nversion = "{bv.current(ROOT)}"',
                                             'name = "spicerack"\nversion = "1.4.9"', 1))
    return tmp_path


@pytest.mark.parametrize("how, want", [("patch", "1.4.10"), ("minor", "1.5.0"), ("major", "2.0.0"), ("1.6.0", "1.6.0")])
def test_bump_moves_every_manifest(tree, how, want):
    assert bv.bump(tree, how) == want
    assert bv.current(tree) == want
    assert f'name = "spicerack"\nversion = "{want}"\n' in (tree / "Cargo.lock").read_text()
    # only the package's own version line moves
    assert (tree / "Cargo.toml").read_text().count(f'"{want}"') == 1


@pytest.mark.parametrize("how", ["1.4.9", "1.3.0", "1.5", "1.5.0-rc1", "huge"])
def test_bump_refuses_a_version_that_does_not_move_forward(tree, how):
    with pytest.raises(SystemExit):
        bv.bump(tree, how)


def test_bump_refuses_disagreeing_manifests(tree):
    p = tree / "pyproject.toml"
    p.write_text(p.read_text().replace('version = "1.4.9"', 'version = "1.4.8"', 1))
    with pytest.raises(SystemExit, match="disagree"):
        bv.bump(tree, "patch")
