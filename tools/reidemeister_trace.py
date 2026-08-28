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
import heapq
import importlib.metadata
import json
import random
from pathlib import Path
from typing import Any

TRACE_FORMAT = "unknotdb-labelled-reidemeister-trace-v0"
TRACE_FORMAT_V1 = "unknotdb-labelled-reidemeister-trace-v1"


def _snappy_version() -> str:
    for distribution in ("snappy", "snappy-manifolds"):
        try:
            return importlib.metadata.version(distribution)
        except importlib.metadata.PackageNotFoundError:
            pass
    return "not-installed"


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


def _endpoint_reindexing(old_state: list[dict[str, Any]], new_state: list[dict[str, Any]]) -> bool:
    """Check fixed-label planar equivalence up to cyclic port renumbering."""
    old = {item["label"]: item for item in old_state}
    new = {item["label"]: item for item in new_state}
    if set(old) != set(new):
        return False
    candidates: dict[str, list[int]] = {}
    for label, item in old.items():
        old_neighbors = [endpoint[0] for endpoint in item["adjacent"]]
        new_neighbors = [endpoint[0] for endpoint in new[label]["adjacent"]]
        candidates[label] = [
            rotation
            for rotation in range(4)
            if all(
                new_neighbors[(slot + rotation) % 4] == neighbor
                for slot, neighbor in enumerate(old_neighbors)
            )
        ]
        if not candidates[label]:
            return False

    rotations: dict[str, int] = {}

    def search() -> bool:
        if len(rotations) == len(old):
            return True
        label = min(
            (item for item in old if item not in rotations),
            key=lambda item: len(candidates[item]),
        )
        for rotation in candidates[label]:
            pending = [(label, rotation)]
            added: list[str] = []
            valid = True
            while pending and valid:
                current, value = pending.pop()
                if current in rotations:
                    valid = rotations[current] == value
                    continue
                if value not in candidates[current]:
                    valid = False
                    continue
                rotations[current] = value
                added.append(current)
                for slot, (peer, peer_slot) in enumerate(old[current]["adjacent"]):
                    target_peer, target_slot = new[current]["adjacent"][
                        (slot + value) % 4
                    ]
                    if target_peer != peer:
                        valid = False
                        break
                    pending.append((peer, (int(target_slot) - int(peer_slot)) % 4))
            if valid and search():
                return True
            for item in added:
                rotations.pop(item, None)
        return False

    return search()


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
            "snappy_version": _snappy_version(),
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
            "snappy_version": _snappy_version(),
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


def extract_best_first_trace(
    word: list[int], max_states: int, max_depth: int
) -> dict[str, Any]:
    """Find an exact RI/RII/RIII trace with bounded best-first search.

    Every search successor is closed under decreasing RI/RII moves.  The
    queue prefers fewer remaining crossings, then fewer RIII moves.  Search
    affects discovery only: the returned certificate is replayed from its
    explicit primitive moves and checkpoints.
    """
    spherogram, Link, simplify, CrossingStrand = _imports()

    def crossing_by_label(link: Any, label: str):
        matches = [item for item in link.crossings if str(item.label) == label]
        if len(matches) != 1:
            raise ValueError(f"expected one crossing labelled {label}")
        return matches[0]

    def exhaust(link: Any, events: list[dict[str, Any]]) -> None:
        original = simplify.reidemeister_I_and_II

        def record(work_link: Any, crossing: Any):
            before = _state_sha256(work_link)
            at = str(crossing.label)
            eliminated, changed = original(work_link, crossing)
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

        simplify.reidemeister_I_and_II = record
        try:
            simplify.basic_simplify(link)
        finally:
            simplify.reidemeister_I_and_II = original

    initial = Link(braid_closure=word)
    initial_events: list[dict[str, Any]] = []
    original_ri_ii = simplify.reidemeister_I_and_II
    original_riii = simplify.reidemeister_III

    def record_initial_ri_ii(work_link: Any, crossing: Any):
        before = _state_sha256(work_link)
        at = str(crossing.label)
        eliminated, changed = original_ri_ii(work_link, crossing)
        if eliminated:
            initial_events.append(
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

    def record_initial_riii(work_link: Any, triple: Any):
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
        initial_events.append(event)
        return result

    simplify.reidemeister_I_and_II = record_initial_ri_ii
    simplify.reidemeister_III = record_initial_riii
    try:
        random.seed(0)
        initial.simplify("level", type_III_limit=5000)
    finally:
        simplify.reidemeister_I_and_II = original_ri_ii
        simplify.reidemeister_III = original_riii
    counter = 0
    queue = [(len(initial.crossings), 0, counter, initial, initial_events)]
    seen = {_state_sha256(initial)}
    explored = 0
    solution = None
    while queue and explored < max_states:
        _, depth, _, link, events = heapq.heappop(queue)
        explored += 1
        if not link.crossings:
            solution = (link, events, depth)
            break
        if depth >= max_depth:
            continue
        possible = sorted(
            simplify.possible_type_III_moves(link),
            key=lambda move: tuple(
                (str(item.crossing.label), int(item.strand_index)) for item in move
            ),
        )
        for move in possible:
            triple_spec = [
                (str(item.crossing.label), int(item.strand_index)) for item in move
            ]
            child = link.copy()
            triple = [
                CrossingStrand(crossing_by_label(child, label), strand_index)
                for label, strand_index in triple_spec
            ]
            child_events = list(events)
            event = {
                "move": "RIII",
                "triple": [[label, index] for label, index in triple_spec],
                "before_sha256": _state_sha256(child),
            }
            simplify.reidemeister_III(child, triple)
            event.update(
                after_sha256=_state_sha256(child),
                remaining_crossings=len(child.crossings),
            )
            child_events.append(event)
            exhaust(child, child_events)
            checkpoint = _state_sha256(child)
            if checkpoint in seen:
                continue
            seen.add(checkpoint)
            counter += 1
            heapq.heappush(
                queue,
                (
                    len(child.crossings),
                    depth + 1,
                    counter,
                    child,
                    child_events,
                ),
            )
    if solution is None:
        best = queue[0][0] if queue else len(initial.crossings)
        raise ValueError(
            f"best-first search exhausted {explored} states; "
            f"best remaining crossings={best}"
        )
    link, events, depth = solution
    trace = {
        "format": TRACE_FORMAT,
        "engine": {
            "spherogram_version": importlib.metadata.version("spherogram"),
            "snappy_version": _snappy_version(),
            "spherogram_module": str(Path(spherogram.__file__).resolve()),
            "algorithm": "bounded-best-first-RIII-with-RI-RII-closure-v0",
            "max_states": max_states,
            "max_depth": max_depth,
            "explored_states": explored,
            "seen_states": len(seen),
            "solution_riii_depth": depth,
        },
        "input": {"ordinary_artin_braid_word": word, "crossings": len(word)},
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


def extract_backtracked_trace(
    word: list[int], seed: int, backtrack_steps: int, type_iii_limit: int
) -> dict[str, Any]:
    """Extract a trace that may contain exact inverse RII moves.

    This is a bounded escape hatch for hard unknot diagrams.  Reverse RII is
    recorded with its two face endpoints and fresh crossing labels; all later
    moves remain ordinary labelled RI/RII/RIII operations.
    """
    spherogram, Link, simplify, _ = _imports()
    link = Link(braid_closure=word)
    events: list[dict[str, Any]] = []
    original_ri_ii = simplify.reidemeister_I_and_II
    original_riii = simplify.reidemeister_III
    original_reverse_ri = simplify.reverse_type_I
    original_reverse_rii = simplify.reverse_type_II
    fresh_index = len(word)
    initial_sha256 = _state_sha256(link)

    def record_orientation_gap(work_link: Any) -> None:
        actual = _state_sha256(work_link)
        expected = events[-1]["after_sha256"] if events else initial_sha256
        if actual != expected:
            events.append(
                {
                    "move": "Orient",
                    "signs": {
                        str(crossing.label): int(crossing.sign)
                        for crossing in work_link.crossings
                    },
                    "state": _raw_state(work_link),
                    "before_sha256": expected,
                    "after_sha256": actual,
                    "remaining_crossings": len(work_link.crossings),
                }
            )

    def record_ri_ii(work_link: Any, crossing: Any):
        record_orientation_gap(work_link)
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
        record_orientation_gap(work_link)
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

    def record_reverse_rii(
        work_link: Any,
        c: Any,
        d: Any,
        _label1: str,
        _label2: str,
        rebuild: bool = False,
    ):
        nonlocal fresh_index
        record_orientation_gap(work_link)
        before = _state_sha256(work_link)
        labels = [f"x{fresh_index}", f"x{fresh_index + 1}"]
        fresh_index += 2
        result = original_reverse_rii(
            work_link, c, d, labels[0], labels[1], rebuild=False
        )
        added = sorted(
            (crossing for crossing in work_link.crossings if str(crossing.label) in labels),
            key=lambda crossing: str(crossing.label),
        )
        events.append(
            {
                "move": "RII+",
                "c": [str(c.crossing.label), int(c.strand_index)],
                "d": [str(d.crossing.label), int(d.strand_index)],
                "added": labels,
                "added_signs": [int(crossing.sign) for crossing in added],
                "state": _raw_state(work_link),
                "before_sha256": before,
                "after_sha256": _state_sha256(work_link),
                "remaining_crossings": len(work_link.crossings),
            }
        )
        return result

    def record_reverse_ri(
        work_link: Any,
        crossing_strand: Any,
        _label: str,
        hand: str,
        rebuild: bool = False,
    ):
        nonlocal fresh_index
        record_orientation_gap(work_link)
        before = _state_sha256(work_link)
        label = f"x{fresh_index}"
        fresh_index += 1
        result = original_reverse_ri(
            work_link, crossing_strand, label, hand, rebuild=False
        )
        added = next(
            crossing for crossing in work_link.crossings if str(crossing.label) == label
        )
        events.append(
            {
                "move": "RI+",
                "anchor": [
                    str(crossing_strand.crossing.label),
                    int(crossing_strand.strand_index),
                ],
                "hand": hand,
                "added": label,
                "added_sign": int(added.sign),
                "state": _raw_state(work_link),
                "before_sha256": before,
                "after_sha256": _state_sha256(work_link),
                "remaining_crossings": len(work_link.crossings),
            }
        )
        return result

    simplify.reidemeister_I_and_II = record_ri_ii
    simplify.reidemeister_III = record_riii

    try:
        random.seed(seed)
        link.simplify("level", type_III_limit=min(100, type_iii_limit))
        simplify.reverse_type_I = record_reverse_ri
        simplify.reverse_type_II = record_reverse_rii
        try:
            link.backtrack(backtrack_steps)
        finally:
            simplify.reverse_type_I = original_reverse_ri
            simplify.reverse_type_II = original_reverse_rii
        record_orientation_gap(link)
        link.simplify("level", type_III_limit=type_iii_limit)
        record_orientation_gap(link)
    finally:
        simplify.reidemeister_I_and_II = original_ri_ii
        simplify.reidemeister_III = original_riii
        simplify.reverse_type_I = original_reverse_ri
        simplify.reverse_type_II = original_reverse_rii
    if link.crossings:
        raise ValueError(
            f"backtracked level simplification stopped with {len(link.crossings)} crossings"
        )
    trace = {
        "format": TRACE_FORMAT_V1,
        "engine": {
            "spherogram_version": importlib.metadata.version("spherogram"),
            "snappy_version": _snappy_version(),
            "spherogram_module": str(Path(spherogram.__file__).resolve()),
            "algorithm": "seeded-level100-backtrack-level-with-inverse-RI-RII-v1",
            "seed": seed,
            "backtrack_steps": backtrack_steps,
            "type_iii_limit": type_iii_limit,
        },
        "input": {"ordinary_artin_braid_word": word, "crossings": len(word)},
        "changed": bool(events),
        "moves": events,
        "final": {
            "crossings": 0,
            "state_sha256": _state_sha256(link),
            "unlinked_unknot_components": int(link.unlinked_unknot_components),
        },
    }
    return trace


def replay_trace(trace: dict[str, Any]) -> dict[str, int]:
    """Replay only the recorded local operations; never call simplify()."""

    if trace.get("format") not in {TRACE_FORMAT, TRACE_FORMAT_V1}:
        raise ValueError(f"unsupported trace format {trace.get('format')!r}")
    _, Link, simplify, CrossingStrand = _imports()
    word = [int(letter) for letter in trace["input"]["ordinary_artin_braid_word"]]
    link = Link(braid_closure=word)
    counts = {"RI": 0, "RII": 0, "RIII": 0, "RI+": 0, "RII+": 0, "Orient": 0}

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
        elif kind == "RII+":
            c_label, c_index = event["c"]
            d_label, d_index = event["d"]
            added = [str(label) for label in event["added"]]
            if len(added) != 2:
                raise ValueError(f"move {index} RII+ does not add two crossings")
            simplify.reverse_type_II(
                link,
                CrossingStrand(crossing_by_label(c_label), int(c_index)),
                CrossingStrand(crossing_by_label(d_label), int(d_index)),
                added[0],
                added[1],
                rebuild=True,
            )
            for label, sign in zip(added, event["added_signs"]):
                crossing_by_label(label).sign = int(sign)
            # Spherogram may re-index local crossing endpoints while updating
            # cached orientations.  The Rust verifier validates the recorded
            # expanded state by applying the inverse RII and requiring the
            # exact pre-move checkpoint.  Python installs that same state for
            # subsequent labelled replay.
            for item in event["state"]:
                crossing = crossing_by_label(item["label"])
                crossing.sign = int(item["sign"])
                for slot, (label, endpoint) in enumerate(item["adjacent"]):
                    crossing.adjacent[slot] = (
                        crossing_by_label(label),
                        int(endpoint),
                    )
        elif kind == "RI+":
            at_label, at_index = event["anchor"]
            added = str(event["added"])
            simplify.reverse_type_I(
                link,
                CrossingStrand(crossing_by_label(at_label), int(at_index)),
                added,
                str(event["hand"]),
                rebuild=False,
            )
            crossing_by_label(added).sign = int(event["added_sign"])
            for item in event["state"]:
                crossing = crossing_by_label(item["label"])
                crossing.sign = int(item["sign"])
                for slot, (label, endpoint) in enumerate(item["adjacent"]):
                    crossing.adjacent[slot] = (
                        crossing_by_label(label),
                        int(endpoint),
                    )
        elif kind == "Orient":
            expected_labels = {str(crossing.label) for crossing in link.crossings}
            signs = {str(label): int(sign) for label, sign in event["signs"].items()}
            if set(signs) != expected_labels or any(sign not in (-1, 1) for sign in signs.values()):
                raise ValueError(f"move {index} Orient has an invalid sign map")
            if not _endpoint_reindexing(_raw_state(link), event["state"]):
                raise ValueError(f"move {index} Orient is not endpoint reindexing")
            for item in event["state"]:
                crossing = crossing_by_label(item["label"])
                crossing.sign = signs[item["label"]]
                for slot, (label, endpoint) in enumerate(item["adjacent"]):
                    crossing.adjacent[slot] = (crossing_by_label(label), int(endpoint))
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
