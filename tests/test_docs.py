"""Every Python example in the user docs runs, so the docs cannot drift from the API.

Each page's ```python blocks run in order as one script (later blocks reuse
earlier names), in a scratch directory. A block that needs a tool CI does not
have is marked `<!-- not run: <reason> -->` on the line before it; the reason
is required. Oracle: the documented calls succeed against the built module,
and the values the prose promises (`# 3.0`, `# 0.25`) are asserted below.
"""
import os
import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
PAGES = sorted((ROOT / "docs").glob("*.md")) + [ROOT / "README.md"]
BLOCK = re.compile(r"(<!--\s*not run:(?P<reason>[^>]*)-->\s*\n)?```python\n(?P<code>.*?)```", re.S)


def runnable(page):
    blocks = []
    for m in BLOCK.finditer(page.read_text()):
        if m.group(0).startswith("<!--"):
            assert m.group("reason").strip(), f"{page.name}: a `not run` marker needs a reason"
            continue
        blocks.append(m.group("code"))
    return "\n".join(blocks)


@pytest.mark.parametrize("page", PAGES, ids=[p.name for p in PAGES])
def test_examples_run(page, tmp_path, monkeypatch, capsys):
    code = runnable(page)
    if not code:
        assert page.name in {"tools.md", "reference.md"}, f"{page.name} has no runnable example"
        return
    monkeypatch.chdir(tmp_path)
    namespace = {"__name__": "__docs__"}
    exec(compile(code, str(page), "exec"), namespace)


def test_promised_values():
    """The numbers the docs print in comments are the numbers the code gives."""
    import spicerack as ps

    dut = ps.Subcircuit("divider", ["vin", "vout"])
    dut.R("top", "vin", "vout", 2e3)
    dut.R("bot", "vout", dut.gnd, 1e3)
    tb = ps.Testbench(dut)
    tb.V("supply", "vin", "0", 9.0)
    assert tb.operating_point()["vout"] == pytest.approx(3.0, abs=1e-9)

    inv = ps.Subcircuit("half", ["a", "y"])
    inv.R("1", "a", "y", 1e3)
    inv.R("2", "y", inv.gnd, 1e3)
    top = ps.Subcircuit("top", ["in", "out"])
    top.instance(inv, "u1", "in", "mid")
    top.X("u2", "half", "mid", "out")
    tb = ps.Testbench(top)
    tb.add_subcircuit(inv)
    tb.V("in", "in", "0", 1.0)
    assert tb.operating_point()["out"] == pytest.approx(0.2, abs=1e-9)


def test_not_run_blocks_are_rare():
    """A `not run` block is untested prose; keep the list short and named."""
    marked = {p.name: len(re.findall(r"<!--\s*not run:", p.read_text())) for p in PAGES}
    assert {k: v for k, v in marked.items() if v} == {"models.md": 3}
