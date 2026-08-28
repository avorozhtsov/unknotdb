#!/usr/bin/env python3
"""Build the durable five-crossing Wang--Zhang cube import corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--extraction", type=Path, required=True)
    parser.add_argument("--terminal-trace", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    extraction = json.loads(args.extraction.read_text())
    trace = json.loads(args.terminal_trace.read_text())
    analysis = extraction["spherogram"]
    base_word = [int(letter) for letter in analysis["braid_word"]]
    positions = [int(position) for position in analysis["marked_braid_positions"]]
    if len(positions) != 5 or len(set(positions)) != 5:
        raise ValueError("extraction does not have five distinct marked positions")
    changed = list(base_word)
    for position in positions:
        changed[position] = -changed[position]
    trace_word = [
        int(letter) for letter in trace["input"]["ordinary_artin_braid_word"]
    ]
    if changed != trace_word:
        raise ValueError("terminal trace is not for the all-five-changed braid")
    corpus = {
        "format": "unknotdb-wang-zhang-five-crossing-cube-v0",
        "source": {
            "paper": "Wang--Zhang arXiv:2507.14265",
            "extraction": str(args.extraction),
            "extraction_sha256": sha256(args.extraction),
            "terminal_trace": str(args.terminal_trace),
            "terminal_trace_sha256": sha256(args.terminal_trace),
        },
        "strands": int(analysis["braid_strands"]),
        "base_word": base_word,
        "marked_positions": positions,
        "terminal_zero_cc_trace": trace,
    }
    args.output.write_text(json.dumps(corpus, indent=2, sort_keys=True) + "\n")
    print(
        json.dumps(
            {
                "output": str(args.output),
                "sha256": sha256(args.output),
                "strands": corpus["strands"],
                "word_length": len(base_word),
                "marked_positions": positions,
                "states": 32,
                "directed_one_cc_edges": 80,
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
