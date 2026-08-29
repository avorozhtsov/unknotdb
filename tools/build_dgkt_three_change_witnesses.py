#!/usr/bin/env python3
"""Compile D/G/K/T ten-crossing claims into braid-native proof witnesses.

The upstream certificates are stated on KnotTheory PD diagrams, whereas the
production graph stores closed braids.  This tool therefore keeps two checks
separate:

* independently reconstruct and verify each upstream PD/Tietze certificate;
* find and replay an exact three-CC RI/RII/RIII witness on the graph's standard
  Spherogram braid for the same named knot.

Only the latter is suitable for import into the braid-native proof graph.  The
upstream certificate remains pinned provenance and an independent cross-check;
it is never treated as an unrecorded PD-to-braid equivalence move.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import itertools
import json
import sqlite3
import time
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any

from build_catalogue_reidemeister_witnesses import normalize_word, rep_key
from reidemeister_trace import extract_seeded_level_trace, extract_trace

FORMAT = "unknotdb-dgkt-three-change-braid-witness-cohort-v0"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def successor(label: int, edge_count: int) -> int:
    return label % edge_count + 1


def change_crossing(labels: Sequence[int], edge_count: int) -> list[int]:
    incoming_under, first_upper, outgoing_under, second_upper = map(int, labels)
    if successor(incoming_under, edge_count) != outgoing_under:
        raise ValueError("PD under-strand orientation is inconsistent")
    if successor(second_upper, edge_count) == first_upper:
        return [second_upper, incoming_under, first_upper, outgoing_under]
    if successor(first_upper, edge_count) == second_upper:
        return [first_upper, outgoing_under, second_upper, incoming_under]
    raise ValueError("PD upper-strand orientation is inconsistent")


def changed_pd(
    pd: Sequence[Sequence[int]], changed_crossings: Iterable[int]
) -> list[list[int]]:
    selected = set(changed_crossings)
    edge_count = 2 * len(pd)
    return [
        change_crossing(labels, edge_count) if index in selected else list(labels)
        for index, labels in enumerate(pd)
    ]


class UnionFind:
    def __init__(self, elements: Iterable[int]) -> None:
        self.parent = {element: element for element in elements}

    def find(self, element: int) -> int:
        if self.parent[element] != element:
            self.parent[element] = self.find(self.parent[element])
        return self.parent[element]

    def union(self, first: int, second: int) -> None:
        first, second = self.find(first), self.find(second)
        if first != second:
            self.parent[max(first, second)] = min(first, second)


def freely_reduce(word: Sequence[int], cyclic: bool = True) -> list[int]:
    reduced: list[int] = []
    for letter in word:
        if reduced and reduced[-1] == -letter:
            reduced.pop()
        else:
            reduced.append(int(letter))
    if cyclic:
        while len(reduced) > 1 and reduced[0] == -reduced[-1]:
            reduced = reduced[1:-1]
    return reduced


def inverse_word(word: Sequence[int]) -> list[int]:
    return [-letter for letter in reversed(word)]


def substitute(
    word: Sequence[int], generator: int, replacement: Sequence[int]
) -> list[int]:
    result: list[int] = []
    inverse = inverse_word(replacement)
    for letter in word:
        if letter == generator:
            result.extend(replacement)
        elif letter == -generator:
            result.extend(inverse)
        else:
            result.append(letter)
    return freely_reduce(result)


def wirtinger_presentation(pd: Sequence[Sequence[int]]) -> dict[str, Any]:
    edge_count = 2 * len(pd)
    labels = range(1, edge_count + 1)
    arcs = UnionFind(labels)
    for crossing in pd:
        arcs.union(int(crossing[1]), int(crossing[3]))
    roots = sorted({arcs.find(label) for label in labels})
    generator_for_root = {root: index + 1 for index, root in enumerate(roots)}
    generator_by_label = {
        label: generator_for_root[arcs.find(label)] for label in labels
    }
    labels_by_generator = {
        str(generator): [
            label for label in labels if generator_by_label[label] == generator
        ]
        for generator in generator_for_root.values()
    }
    relators = []
    for incoming_under, first_upper, outgoing_under, second_upper in pd:
        incoming = generator_by_label[int(incoming_under)]
        outgoing = generator_by_label[int(outgoing_under)]
        upper = generator_by_label[int(first_upper)]
        if successor(int(second_upper), edge_count) == int(first_upper):
            relator = [-outgoing, -upper, incoming, upper]
        elif successor(int(first_upper), edge_count) == int(second_upper):
            relator = [-outgoing, upper, incoming, -upper]
        else:
            raise ValueError("PD upper-strand orientation is inconsistent")
        relators.append(freely_reduce(relator))
    return {
        "generators": sorted(generator_for_root.values()),
        "labels_by_generator": labels_by_generator,
        "relators": relators,
    }


def tietze_reduce(presentation: dict[str, Any]) -> dict[str, Any]:
    generators = set(map(int, presentation["generators"]))
    relators = [freely_reduce(word) for word in presentation["relators"]]
    relators = [word for word in relators if word]
    steps = []
    while True:
        choice = None
        for relator_index, relator in enumerate(relators):
            for generator in sorted(generators):
                positions = [
                    index
                    for index, letter in enumerate(relator)
                    if abs(letter) == generator
                ]
                if len(positions) == 1:
                    choice = (relator_index, generator, positions[0])
                    break
            if choice is not None:
                break
        if choice is None:
            break
        relator_index, generator, position = choice
        relator = relators[relator_index]
        rotated = relator[position:] + relator[:position]
        replacement = (
            inverse_word(rotated[1:])
            if rotated[0] == generator
            else list(rotated[1:])
        )
        replacement = freely_reduce(replacement, cyclic=False)
        steps.append(
            {
                "eliminated_generator": generator,
                "using_relator": relator,
                "replacement_word": replacement,
            }
        )
        relators = [
            substitute(other, generator, replacement)
            for index, other in enumerate(relators)
            if index != relator_index
        ]
        relators = [word for word in relators if word]
        generators.remove(generator)
    return {
        "remaining_generators": sorted(generators),
        "remaining_relators": relators,
        "elimination_steps": steps,
        "reduces_to_infinite_cyclic": len(generators) == 1 and not relators,
    }


def verify_upstream_entry(entry: dict[str, Any]) -> dict[str, Any]:
    certificate = entry["three_change_unknot_certificate"]
    positions = [int(position) - 1 for position in certificate["positions_1_based"]]
    if len(positions) != 3 or len(set(positions)) != 3:
        raise ValueError(f"{entry['knot']}: expected three distinct crossing positions")
    final_pd = certificate["changed_pd"]
    source_pd = changed_pd(final_pd, positions)
    if changed_pd(source_pd, positions) != final_pd:
        raise ValueError(f"{entry['knot']}: crossing-change involution failed")
    presentation = wirtinger_presentation(final_pd)
    if presentation != certificate["wirtinger_presentation"]:
        raise ValueError(f"{entry['knot']}: Wirtinger presentation mismatch")
    reduction = tietze_reduce(presentation)
    if reduction != certificate["tietze_reduction"]:
        raise ValueError(f"{entry['knot']}: Tietze reduction mismatch")
    if not reduction["reduces_to_infinite_cyclic"]:
        raise ValueError(f"{entry['knot']}: terminal group is not infinite cyclic")
    return {
        "positions_1_based": certificate["positions_1_based"],
        "source_pd": source_pd,
        "changed_pd": final_pd,
        "tietze_steps": len(reduction["elimination_steps"]),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--source-url", required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--max-type-iii", type=int, default=250)
    parser.add_argument("--seeded-fallback-seeds", type=int, default=4)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or args.summary.exists():
        raise FileExistsError("output artifact already exists")

    try:
        from spherogram import Link
    except ImportError as error:
        raise SystemExit("Spherogram is required") from error

    upstream = json.loads(args.source.read_text())
    graph = sqlite3.connect(f"file:{args.graph}?mode=ro", uri=True)
    started = time.monotonic()
    results: list[dict[str, Any]] = []
    try:
        for entry in upstream["knots"]:
            audit = verify_upstream_entry(entry)
            knot = str(entry["knot"])
            source_word = list(map(int, Link(knot).braid_word()))
            strands = max(map(abs, source_word)) + 1
            word, mirrored = normalize_word(source_word)
            key = rep_key(strands, word)
            graph_row = graph.execute(
                "SELECT n.node_id,n.u_upper_bound FROM node_keys k "
                "JOIN nodes n USING(node_id) WHERE k.rep_key=?",
                (key,),
            ).fetchone()
            attempts = 0
            seeded_attempts = 0
            witness = None
            last_error = None
            for positions in itertools.combinations(range(len(word)), 3):
                attempts += 1
                changed = word.copy()
                for position in positions:
                    changed[position] = -changed[position]
                try:
                    trace = extract_trace(changed, args.max_type_iii)
                except ValueError as error:
                    last_error = str(error)
                    continue
                witness = {
                    "crossing_change_positions": list(positions),
                    "post_cc_word": changed,
                    "zero_cc_trace": trace,
                }
                break
            if witness is None:
                for positions in itertools.combinations(range(len(word)), 3):
                    changed = word.copy()
                    for position in positions:
                        changed[position] = -changed[position]
                    for seed in range(args.seeded_fallback_seeds):
                        seeded_attempts += 1
                        try:
                            trace = extract_seeded_level_trace(
                                changed, seed, args.max_type_iii
                            )
                        except ValueError as error:
                            last_error = str(error)
                            continue
                        witness = {
                            "crossing_change_positions": list(positions),
                            "post_cc_word": changed,
                            "zero_cc_trace": trace,
                        }
                        break
                    if witness is not None:
                        break
            result = {
                "knot_id": f"knot:{knot}",
                "catalogue_exact_u": 3,
                "standard_braid": {
                    "strands": strands,
                    "word": source_word,
                    "normalized_word": word,
                    "normalization_mirrored": mirrored,
                    "rep_key": key.hex(),
                },
                "graph": {
                    "node_id": graph_row[0] if graph_row else None,
                    "u_upper": graph_row[1] if graph_row else None,
                    "exact_key_hit": graph_row is not None,
                },
                "source_pd_certificate": audit,
                "search": {
                    "attempts": attempts,
                    "scheduler_attempts": seeded_attempts,
                    "status": "witness" if witness else "miss",
                    "last_error": None if witness else last_error,
                },
                "witness": witness,
            }
            results.append(result)
            print(
                f"knot:{knot}\t{result['search']['status']}\t"
                f"attempts={attempts}\tgraph_hit={graph_row is not None}",
                flush=True,
            )
    finally:
        graph.close()

    artifact = {
        "format": FORMAT,
        "provenance": {
            "authors": "Dranowski--Guo--Kabkov--Tubbenhauer",
            "repository": "https://github.com/dtubbenhauer/unknot",
            "license": "Unlicense",
            "source_url": args.source_url,
            "source_sha256": file_sha256(args.source),
            "source_method": upstream.get("method"),
            "scope_note": (
                "Upstream proves minimal-diagram unknotting number 3; the "
                "braid-native traces below independently prove only U_upper <= 3."
            ),
        },
        "inputs": {
            "graph": str(args.graph),
            "graph_sha256": file_sha256(args.graph),
            "max_type_iii": args.max_type_iii,
            "seeded_fallback_seeds": args.seeded_fallback_seeds,
        },
        "summary": {
            "selected": len(results),
            "upstream_pd_certificates_verified": len(results),
            "witnesses": sum(item["witness"] is not None for item in results),
            "misses": sum(item["witness"] is None for item in results),
            "exact_graph_key_hits": sum(
                item["graph"]["exact_key_hit"] for item in results
            ),
            "elapsed_seconds": time.monotonic() - started,
        },
        "results": results,
    }
    args.output.write_text(json.dumps(artifact, indent=2) + "\n")
    with args.summary.open("w", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(
            (
                "knot_id",
                "old_u",
                "status",
                "attempts",
                "seeded_attempts",
                "cc_positions",
                "zero_ri",
                "zero_rii",
                "zero_riii",
                "upstream_pd_positions",
                "upstream_tietze_steps",
            )
        )
        for item in results:
            witness = item["witness"]
            moves = witness["zero_cc_trace"]["moves"] if witness else []
            writer.writerow(
                (
                    item["knot_id"],
                    item["graph"]["u_upper"],
                    item["search"]["status"],
                    item["search"]["attempts"],
                    item["search"]["scheduler_attempts"],
                    ",".join(map(str, witness["crossing_change_positions"]))
                    if witness
                    else "",
                    sum(move["move"] == "RI" for move in moves),
                    sum(move["move"] == "RII" for move in moves),
                    sum(move["move"] == "RIII" for move in moves),
                    ",".join(
                        map(str, item["source_pd_certificate"]["positions_1_based"])
                    ),
                    item["source_pd_certificate"]["tietze_steps"],
                )
            )
    print(json.dumps(artifact["summary"], sort_keys=True))


if __name__ == "__main__":
    main()
