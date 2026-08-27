#!/usr/bin/env python3
"""Extract and independently replay exact RI/RII/RIII unknot traces.

This optional tool deliberately keeps Spherogram outside the Rust runtime.  An
extracted trace contains no appeal to ``Link.simplify`` during validation: the
validator reconstructs the braid closure and replays each named local move,
checking a SHA-256 checkpoint before and after every operation.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import random
from pathlib import Path
from typing import Any

TRACE_FORMAT = "unknotdb-labelled-reidemeister-trace-v0"


def _imports():
    try:
        import spherogram
        from spherogram import Link
        from spherogram.links import simplify
        from spherogram.links.links import CrossingStrand
    except ImportError as error:
        raise SystemExit(
            "Spherogram is required; run with a Python environment containing SnapPy"
        ) from error
    return spherogram, Link, simplify, CrossingStrand


def _raw_state(link: Any) -> list[dict[str, Any]]:
    """A label-preserving combinatorial diagram checkpoint.

    Strand labels are intentionally excluded: they are cached presentation
    data.  Adjacency, local endpoint indices, and crossing signs determine the
    labelled diagram needed by the local-move replay.
    """

    state = []
    for crossing in sorted(link.crossings, key=lambda item: str(item.label)):
        state.append(
            {
                "label": str(crossing.label),
                "sign": int(crossing.sign),
                "adjacent": [
                    [str(other.label), int(index)]
                    for other, index in crossing.adjacent
                ],
            }
        )
    return state


def _state_sha256(link: Any) -> str:
    encoded = json.dumps(
        _raw_state(link), sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def extract_trace(word: list[int], type_iii_limit: int) -> dict[str, Any]:
    spherogram, Link, simplify, _ = _imports()
    link = Link(braid_closure=word)
    events: list[dict[str, Any]] = []

    def try_ri_ii(crossing: Any) -> bool:
        before = _state_sha256(link)
        at = str(crossing.label)
        eliminated, _ = simplify.reidemeister_I_and_II(link, crossing)
        if eliminated:
            events.append(
                {
                    "move": "RI" if len(eliminated) == 1 else "RII",
                    "at": at,
                    "eliminated": sorted(str(item.label) for item in eliminated),
                    "before_sha256": before,
                    "after_sha256": _state_sha256(link),
                    "remaining_crossings": len(link.crossings),
                }
            )
        return bool(eliminated)

    def exhaust_ri_ii() -> None:
        # Restart after each accepted move.  This gives a stable schedule and
        # avoids depending on Python set iteration or object addresses.
        while True:
            for crossing in sorted(link.crossings, key=lambda item: str(item.label)):
                if try_ri_ii(crossing):
                    break
            else:
                return

    def apply_riii(triple: Any) -> None:
        event = {
            "move": "RIII",
            "triple": [
                [str(item.crossing.label), int(item.strand_index)] for item in triple
            ],
            "before_sha256": _state_sha256(link),
        }
        simplify.reidemeister_III(link, triple)
        event.update(
            after_sha256=_state_sha256(link),
            remaining_crossings=len(link.crossings),
        )
        events.append(event)

    exhaust_ri_ii()
    seen = {_state_sha256(link)}
    for _ in range(type_iii_limit):
        if not link.crossings:
            break
        possible = simplify.possible_type_III_moves(link)
        if not possible:
            break
        triple = min(
            possible,
            key=lambda move: tuple(
                (str(item.crossing.label), int(item.strand_index)) for item in move
            ),
        )
        apply_riii(triple)
        exhaust_ri_ii()
        checkpoint = _state_sha256(link)
        if checkpoint in seen:
            raise ValueError("deterministic level simplification entered a cycle")
        seen.add(checkpoint)

    if len(link.crossings) != 0:
        raise ValueError(
            f"level simplification stopped with {len(link.crossings)} crossings"
        )
    trace = {
        "format": TRACE_FORMAT,
        "engine": {
            "spherogram_version": importlib.metadata.version("spherogram"),
            "snappy_version": importlib.metadata.version("snappy"),
            "spherogram_module": str(Path(spherogram.__file__).resolve()),
            "algorithm": "lexicographic-RI/RII-then-RIII-v0",
            "type_iii_limit": type_iii_limit,
        },
        "input": {
            "ordinary_artin_braid_word": word,
            "crossings": len(word),
        },
        "changed": bool(events),
        "moves": events,
        "final": {
            "crossings": 0,
            "state_sha256": _state_sha256(link),
            "unlinked_unknot_components": int(link.unlinked_unknot_components),
        },
    }
    replay_trace(trace)
    return trace


def extract_seeded_level_trace(
    word: list[int], seed: int, type_iii_limit: int
) -> dict[str, Any]:
    """Extract one fixed trace from Spherogram's seeded level scheduler.

    The durable certificate remains independent of the scheduler: replay uses
    only the emitted local operations and checkpoints.  This fallback is useful
    when the lexicographic RIII route cycles without simplifying.
    """

    spherogram, Link, simplify, _ = _imports()
    link = Link(braid_closure=word)
    events: list[dict[str, Any]] = []
    original_ri_ii = simplify.reidemeister_I_and_II
    original_riii = simplify.reidemeister_III

    def record_ri_ii(work_link: Any, crossing: Any):
        before = _state_sha256(work_link)
        at = str(crossing.label)
        eliminated, changed = original_ri_ii(work_link, crossing)
        if eliminated:
            events.append(
                {
                    "move": "RI" if len(eliminated) == 1 else "RII",
                    "at": at,
                    "eliminated": sorted(str(item.label) for item in eliminated),
                    "before_sha256": before,
                    "after_sha256": _state_sha256(work_link),
                    "remaining_crossings": len(work_link.crossings),
                }
            )
        return eliminated, changed

    def record_riii(work_link: Any, triple: Any):
        event = {
            "move": "RIII",
            "triple": [
                [str(item.crossing.label), int(item.strand_index)] for item in triple
            ],
            "before_sha256": _state_sha256(work_link),
        }
        result = original_riii(work_link, triple)
        event.update(
            after_sha256=_state_sha256(work_link),
            remaining_crossings=len(work_link.crossings),
        )
        events.append(event)
        return result

    simplify.reidemeister_I_and_II = record_ri_ii
    simplify.reidemeister_III = record_riii
    try:
        random.seed(seed)
        changed = link.simplify("level", type_III_limit=type_iii_limit)
    finally:
        simplify.reidemeister_I_and_II = original_ri_ii
        simplify.reidemeister_III = original_riii
    if len(link.crossings) != 0:
        raise ValueError(
            f"seeded level simplification stopped with {len(link.crossings)} crossings"
        )
    trace = {
        "format": TRACE_FORMAT,
        "engine": {
            "spherogram_version": importlib.metadata.version("spherogram"),
            "snappy_version": importlib.metadata.version("snappy"),
            "spherogram_module": str(Path(spherogram.__file__).resolve()),
            "algorithm": "seeded-Link.simplify(level)-v0",
            "seed": seed,
            "type_iii_limit": type_iii_limit,
        },
        "input": {"ordinary_artin_braid_word": word, "crossings": len(word)},
        "changed": bool(changed),
        "moves": events,
        "final": {
            "crossings": 0,
            "state_sha256": _state_sha256(link),
            "unlinked_unknot_components": int(link.unlinked_unknot_components),
        },
    }
    replay_trace(trace)
    return trace


def replay_trace(trace: dict[str, Any]) -> dict[str, int]:
    """Replay only the recorded local operations; never call simplify()."""

    if trace.get("format") != TRACE_FORMAT:
        raise ValueError(f"unsupported trace format {trace.get('format')!r}")
    _, Link, simplify, CrossingStrand = _imports()
    word = [int(letter) for letter in trace["input"]["ordinary_artin_braid_word"]]
    link = Link(braid_closure=word)
    counts = {"RI": 0, "RII": 0, "RIII": 0}

    def crossing_by_label(label: str):
        matches = [item for item in link.crossings if str(item.label) == label]
        if len(matches) != 1:
            raise ValueError(f"expected one crossing labelled {label}, found {len(matches)}")
        return matches[0]

    for index, event in enumerate(trace["moves"]):
        actual_before = _state_sha256(link)
        if actual_before != event["before_sha256"]:
            raise ValueError(
                f"move {index} input checkpoint mismatch: "
                f"{actual_before} != {event['before_sha256']}"
            )
        kind = event["move"]
        if kind in ("RI", "RII"):
            eliminated, _ = simplify.reidemeister_I_and_II(
                link, crossing_by_label(event["at"])
            )
            actual_eliminated = sorted(str(item.label) for item in eliminated)
            if actual_eliminated != event["eliminated"]:
                raise ValueError(
                    f"move {index} eliminated {actual_eliminated}, "
                    f"expected {event['eliminated']}"
                )
            expected_count = 1 if kind == "RI" else 2
            if len(eliminated) != expected_count:
                raise ValueError(f"move {index} is not a valid {kind}")
        elif kind == "RIII":
            triple = [
                CrossingStrand(crossing_by_label(label), int(strand_index))
                for label, strand_index in event["triple"]
            ]
            legal = {
                tuple((str(item.crossing.label), int(item.strand_index)) for item in move)
                for move in simplify.possible_type_III_moves(link)
            }
            descriptor = tuple((label, int(strand_index)) for label, strand_index in event["triple"])
            if descriptor not in legal:
                raise ValueError(f"move {index} is not an available RIII move")
            simplify.reidemeister_III(link, triple)
        else:
            raise ValueError(f"move {index} has unknown kind {kind!r}")
        counts[kind] += 1
        actual_after = _state_sha256(link)
        if actual_after != event["after_sha256"]:
            raise ValueError(
                f"move {index} output checkpoint mismatch: "
                f"{actual_after} != {event['after_sha256']}"
            )
        if len(link.crossings) != int(event["remaining_crossings"]):
            raise ValueError(f"move {index} crossing count mismatch")

    if len(link.crossings) != 0:
        raise ValueError(f"trace ended with {len(link.crossings)} crossings")
    if _state_sha256(link) != trace["final"]["state_sha256"]:
        raise ValueError("final diagram checkpoint mismatch")
    return counts


def _parse_word(value: str) -> list[int]:
    return [int(item) for item in value.replace(",", " ").split()]


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    extract = subparsers.add_parser("extract")
    extract.add_argument("--word", required=True, type=_parse_word)
    extract.add_argument("--type-iii-limit", type=int, default=5000)
    extract.add_argument("--seeded-level", type=int)
    extract.add_argument("--output", required=True, type=Path)
    replay = subparsers.add_parser("replay")
    replay.add_argument("trace", type=Path)
    args = parser.parse_args()

    if args.command == "extract":
        trace = (
            extract_seeded_level_trace(
                args.word, args.seeded_level, args.type_iii_limit
            )
            if args.seeded_level is not None
            else extract_trace(args.word, args.type_iii_limit)
        )
        args.output.write_text(json.dumps(trace, indent=2) + "\n")
        counts = replay_trace(trace)
        print(f"wrote {args.output}; replay OK; {counts}")
    else:
        trace = json.loads(args.trace.read_text())
        counts = replay_trace(trace)
        print(f"replay OK; {counts}")


if __name__ == "__main__":
    main()
