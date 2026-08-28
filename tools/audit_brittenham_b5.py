#!/usr/bin/env python3
"""Audit the tempting, but false, direct five-CC B5 shortcut.

Brittenham--Hermiller publish a 20-letter B5 representative of
``7_1 # mirror(7_1)`` and two crossing changes at positions 0 and 1 which
reach ``K14a18636``.  Their remaining three crossing changes occur only after
changing diagrams.  This tool exhausts the 18 choose 3 ways of pretending
that those three changes can also be made in the original braid chart.

The determinant is computed exactly from the Fox-colouring matrix of the
Spherogram diagram.  Since the unknot has determinant one, a candidate with
another determinant cannot be accepted as a terminal proof-graph state.
"""

from __future__ import annotations

import argparse
import hashlib
import itertools
import json
from collections import Counter
from pathlib import Path
from typing import Any

FORMAT = "unknotdb-brittenham-b5-direct-five-cc-audit-v0"
SOURCE_WORD = [
    1,
    -4,
    2,
    3,
    3,
    3,
    2,
    3,
    2,
    2,
    4,
    -3,
    -3,
    -3,
    -3,
    -1,
    -3,
    -2,
    -3,
    -3,
]
FIXED_POSITIONS = (0, 1)


def _imports() -> Any:
    try:
        from spherogram import Link
    except ImportError as error:
        raise SystemExit("Spherogram is required for this audit") from error
    return Link


def _bareiss_determinant(matrix: list[list[int]]) -> int:
    """Return an exact integer determinant using fraction-free elimination."""

    size = len(matrix)
    if size == 0:
        return 1
    work = [row[:] for row in matrix]
    sign = 1
    denominator = 1
    for pivot_column in range(size - 1):
        pivot_row = next(
            (row for row in range(pivot_column, size) if work[row][pivot_column]),
            None,
        )
        if pivot_row is None:
            return 0
        if pivot_row != pivot_column:
            work[pivot_column], work[pivot_row] = (work[pivot_row], work[pivot_column])
            sign = -sign
        pivot = work[pivot_column][pivot_column]
        for row in range(pivot_column + 1, size):
            for column in range(pivot_column + 1, size):
                numerator = (
                    work[row][column] * pivot
                    - work[row][pivot_column] * work[pivot_column][column]
                )
                work[row][column] = numerator // denominator
        denominator = pivot
        for row in range(pivot_column + 1, size):
            work[row][pivot_column] = 0
    return sign * work[-1][-1]


def knot_determinant(word: list[int]) -> int:
    """Compute |det| from a reduced Fox-colouring presentation matrix."""

    link = _imports()(braid_closure=word)
    if len(link.link_components) != 1:
        raise ValueError("determinant audit requires a one-component closure")
    pieces = link._pieces()  # Spherogram's arcs between undercrossings.
    matrix = [[0 for _ in pieces] for _ in link.crossings]
    for crossing_index, crossing in enumerate(link.crossings):
        for endpoint in range(4):
            piece_index = next(
                index
                for index, piece in enumerate(pieces)
                if (crossing, endpoint) in piece
            )
            matrix[crossing_index][piece_index] += 1 if endpoint % 2 else -1
    # Delete one row and one column, exactly as for the standard colourability
    # determinant.  All diagrams in this audit are one-component closures.
    minor = [row[1:] for row in matrix[1:]]
    return abs(_bareiss_determinant(minor))


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", required=True, type=Path)
    parser.add_argument("--tsv", required=True, type=Path)
    parser.add_argument("--source-tex", type=Path)
    args = parser.parse_args()

    rows = []
    for variable_positions in itertools.combinations(range(2, 20), 3):
        changed_word = SOURCE_WORD[:]
        positions = FIXED_POSITIONS + variable_positions
        for position in positions:
            changed_word[position] = -changed_word[position]
        rows.append(
            {
                "variable_positions": list(variable_positions),
                "all_positions": list(positions),
                "determinant": knot_determinant(changed_word),
                "word": changed_word,
            }
        )

    distribution = Counter(row["determinant"] for row in rows)
    determinant_one = [row for row in rows if row["determinant"] == 1]
    source: dict[str, Any] = {
        "paper": "Brittenham--Hermiller, arXiv:2506.24088",
        "source_url": "https://export.arxiv.org/e-print/2506.24088",
        "published_word": SOURCE_WORD,
        "published_fixed_positions": list(FIXED_POSITIONS),
    }
    if args.source_tex is not None:
        source.update(
            source_tex_name=args.source_tex.name,
            source_tex_sha256=sha256(args.source_tex),
        )
    report = {
        "format": FORMAT,
        "source": source,
        "candidate_contract": {
            "variable_positions": list(range(2, 20)),
            "choose": 3,
            "expected_candidates": 816,
            "terminal_necessary_condition": "determinant == 1",
        },
        "result": {
            "tested": len(rows),
            "determinant_one": len(determinant_one),
            "direct_five_cc_terminal_exists": bool(determinant_one),
            "conclusion": (
                "no direct five-CC cube exists in this B5 chart"
                if not determinant_one
                else "determinant prefilter survivors require exact unknot replay"
            ),
            "determinant_distribution": {
                str(value): count for value, count in sorted(distribution.items())
            },
        },
        "rows": rows,
    }
    args.json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    with args.tsv.open("w") as output:
        output.write("variable_positions\tall_positions\tdeterminant\tword\n")
        for row in rows:
            output.write(
                f"{','.join(map(str, row['variable_positions']))}\t"
                f"{','.join(map(str, row['all_positions']))}\t"
                f"{row['determinant']}\t{','.join(map(str, row['word']))}\n"
            )
    print(
        json.dumps(
            {
                "format": FORMAT,
                "tested": len(rows),
                "determinant_one": len(determinant_one),
                "json": str(args.json),
                "json_sha256": sha256(args.json),
                "tsv": str(args.tsv),
                "tsv_sha256": sha256(args.tsv),
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
