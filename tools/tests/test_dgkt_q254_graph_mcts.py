from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

TOOL = Path(__file__).parents[1] / "run_dgkt_q254_graph_mcts.py"
SPEC = importlib.util.spec_from_file_location("dgkt_q254_graph_mcts", TOOL)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


def test_normalized_key_matches_rust_reference_vector() -> None:
    word = [-1, 2, -3, 2, -1, 2, 2, -1, -3, -2, -2, -1, -1]
    _, key = MODULE.normalized(word, 4)
    assert key.hex() == "92e4874c12080d2e031a7e1b8be7790bccacef3aef7df751f38db6bae0b156ce"


def test_reducer_uses_destabilize_before_cyclic_r2() -> None:
    key, program = MODULE.reduce_to_key([2, 1, -1], 3)
    _, expected = MODULE.normalized([], 2)
    assert key == expected
    assert [step["kind"] for step in program] == [
        "normalize",
        "destabilize",
        "reduce",
        "normalize",
    ]
