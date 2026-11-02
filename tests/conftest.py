import shutil
import sys
from pathlib import Path

import pytest


PYTHON_SRC = Path(__file__).resolve().parents[1] / "python"
if str(PYTHON_SRC) not in sys.path:
    sys.path.insert(0, str(PYTHON_SRC))


def pytest_configure(config):
    config.addinivalue_line("markers", "needs(tool): runs `tool`; fails, never skips, when it is not on PATH")


@pytest.fixture(autouse=True)
def _needs(request):
    """Unsupported fails, never skips: a missing simulator is a failure."""
    for mark in request.node.iter_markers("needs"):
        tool = mark.args[0]
        if shutil.which(tool) is None:
            pytest.fail(f"{tool} not on PATH: run the tests in `nix develop .#testing`")
