#!/usr/bin/env python3
"""Build exact non-knot fingerprint postings for source braid representations."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path
from typing import Any


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical_json(value: Any) -> str:
    return json.dumps(value, separators=(",", ":"), sort_keys=True)


def value_hash(fingerprint_id: str, value_type: str, value: str) -> bytes:
    return hashlib.sha256(
        b"UNKNOTDB_BRAID_FINGERPRINT_V0\0"
        + fingerprint_id.encode()
        + b"\0"
        + value_type.encode()
        + b"\0"
        + value.encode()
    ).digest()


def load_representations(paths: list[Path]) -> dict[str, tuple[tuple[int, ...], int]]:
    result: dict[str, tuple[tuple[int, ...], int]] = {}
    for path in paths:
        for entry in json.loads(path.read_text())["entries"]:
            candidate = (tuple(map(int, entry["word"])), int(entry["strands"]))
            previous = result.setdefault(entry["representation_id"], candidate)
            if previous != candidate:
                raise ValueError(f"conflicting braid payload for {entry['representation_id']}")
    return result


def permutation_data(word: tuple[int, ...], strands: int) -> tuple[list[int], list[int]]:
    permutation = list(range(strands))
    for letter in word:
        index = abs(letter) - 1
        permutation[index], permutation[index + 1] = permutation[index + 1], permutation[index]
    seen: set[int] = set()
    cycles = []
    for start in range(strands):
        if start in seen:
            continue
        current = start
        length = 0
        while current not in seen:
            seen.add(current)
            current = permutation[current]
            length += 1
        cycles.append(length)
    return permutation, sorted(cycles, reverse=True)


def polynomial_pairs(polynomial: dict[int, int]) -> list[list[int]]:
    return [[exponent, polynomial[exponent]] for exponent in sorted(polynomial)]


def polynomial_sum(polynomials: list[dict[int, int]]) -> dict[int, int]:
    result: dict[int, int] = {}
    for polynomial in polynomials:
        for exponent, coefficient in polynomial.items():
            result[exponent] = result.get(exponent, 0) + coefficient
            if result[exponent] == 0:
                del result[exponent]
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--maps", type=Path, required=True)
    parser.add_argument("--identifications", type=Path, required=True)
    parser.add_argument("--graph-snapshot", type=Path, required=True)
    parser.add_argument("--corpus-json", type=Path, action="append", required=True)
    parser.add_argument("--rf-src", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")

    representations = load_representations(args.corpus_json)
    maps = sqlite3.connect(f"file:{args.maps}?mode=ro", uri=True)
    maps_meta = dict(maps.execute("SELECT key,value FROM meta"))
    graph_sha256 = file_sha256(args.graph_snapshot)
    if maps_meta["graph_snapshot_sha256"] != graph_sha256:
        raise ValueError("lookup maps and graph snapshot hashes differ")
    representation_maps = {
        identity: (key, node_id, u_upper, None, None)
        for identity, key, node_id, u_upper in maps.execute(
            """
            SELECT g.representation_id,g.stopping_key,g.graph_node_id,g.graph_u_upper
            FROM representation_graph_map g
            """
        )
    }
    maps.close()

    identifications = sqlite3.connect(f"file:{args.identifications}?mode=ro", uri=True)
    identification_meta = dict(identifications.execute("SELECT key,value FROM meta"))
    if identification_meta["lookup_maps_sha256"] != file_sha256(args.maps):
        raise ValueError("identification sidecar and lookup maps hashes differ")
    for identity, knot_id, evidence_class in identifications.execute(
        """
        SELECT representation_id,knot_id,evidence_class
        FROM effective_representation_knot_map
        """
    ):
        key, node_id, u_upper, _, _ = representation_maps[identity]
        representation_maps[identity] = (
            key,
            node_id,
            u_upper,
            knot_id,
            evidence_class,
        )
    identifications.close()

    graph = sqlite3.connect(f"file:{args.graph_snapshot}?mode=ro", uri=True)
    policy_stops = {
        int(node_id): ("preferred_cc" if kind == 0 else "terminal", action)
        for node_id, kind, action in graph.execute(
            "SELECT node_id,stop_kind,preferred_cc_action FROM policy_stops"
        )
    }
    graph.close()

    sys.path.insert(0, str(args.rf_src))
    from rf_knots.invariants import _burau  # type: ignore[attr-defined]

    definitions = {
        "braid_strands": (
            "integer",
            "representation",
            "exact-representation",
            "ranking-only",
            "Number of strands in this braid encoding",
        ),
        "word_length": (
            "integer",
            "representation",
            "exact-representation",
            "ranking-only",
            "Number of braid letters",
        ),
        "writhe": (
            "integer",
            "braid-conjugacy",
            "braid-relations-and-fixed-strand-conjugation",
            "never-reject-knot-match",
            "Signed exponent sum; changes under Markov stabilization",
        ),
        "absolute_writhe": (
            "integer",
            "braid-conjugacy",
            "braid-relations-fixed-strand-conjugation-and-mirror",
            "never-reject-knot-match",
            "Absolute signed exponent sum",
        ),
        "representation_l10": (
            "integer",
            "representation",
            "exact-representation",
            "ranking-only",
            "10*strands + word_length; no crossing-change term",
        ),
        "generator_signed_histogram": (
            "json",
            "representation",
            "cyclic-origin-and-far-commutation-only",
            "ranking-only",
            "Positive/negative occurrence counts for each Artin generator",
        ),
        "strand_permutation": (
            "json",
            "braid-element",
            "braid-relations",
            "never-reject-knot-match",
            "Permutation induced on labelled strands",
        ),
        "permutation_cycle_type": (
            "json",
            "braid-conjugacy",
            "braid-relations-and-fixed-strand-conjugation",
            "never-reject-knot-match",
            "Sorted cycle lengths of the induced strand permutation",
        ),
        "reduced_burau_trace": (
            "laurent-pairs-json",
            "braid-conjugacy",
            "braid-relations-and-fixed-strand-conjugation",
            "never-reject-knot-match",
            "Trace of the exact reduced Burau matrix over Z[t,t^-1]",
        ),
        "reduced_burau_trace_t_minus_1": (
            "integer",
            "braid-conjugacy",
            "braid-relations-and-fixed-strand-conjugation",
            "never-reject-knot-match",
            "Reduced Burau trace evaluated exactly at t=-1",
        ),
        "reduced_burau_matrix_sha256": (
            "sha256-hex",
            "braid-element",
            "braid-relations",
            "candidate-bucketing-only",
            "Hash of the complete exact reduced Burau matrix",
        ),
        "snapshot_u_upper": (
            "integer",
            "graph-snapshot",
            "exact-graph-snapshot-hash-only",
            "ranking-only",
            "Current replay-validated graph upper bound, not a knot invariant",
        ),
        "q254_stop_kind": (
            "text",
            "policy-checkpoint",
            "exact-model-adapter-and-stopping-key-only",
            "ranking-only",
            "Terminal or preferred crossing-change policy stop",
        ),
        "q254_preferred_cc_u63": (
            "integer",
            "policy-checkpoint",
            "exact-model-adapter-and-stopping-key-only",
            "ranking-only",
            "Encoded preferred crossing-change action at the stopping point",
        ),
    }

    rows: list[tuple[str, str, str, str, bytes]] = []
    for index, (identity, (word, strands)) in enumerate(sorted(representations.items()), 1):
        if identity not in representation_maps:
            raise ValueError(f"representation absent from lookup maps: {identity}")
        histogram = [
            [generator, word.count(generator), word.count(-generator)]
            for generator in range(1, strands)
        ]
        permutation, cycle_type = permutation_data(word, strands)
        matrix = _burau(word, strands)
        trace = polynomial_sum([matrix[i][i] for i in range(len(matrix))])
        matrix_json = canonical_json(
            [[polynomial_pairs(entry) for entry in row] for row in matrix]
        )
        values = {
            "braid_strands": str(strands),
            "word_length": str(len(word)),
            "writhe": str(sum(word)),
            "absolute_writhe": str(abs(sum(word))),
            "representation_l10": str(10 * strands + len(word)),
            "generator_signed_histogram": canonical_json(histogram),
            "strand_permutation": canonical_json(permutation),
            "permutation_cycle_type": canonical_json(cycle_type),
            "reduced_burau_trace": canonical_json(polynomial_pairs(trace)),
            "reduced_burau_trace_t_minus_1": str(
                sum(coefficient * (-1) ** (exponent % 2) for exponent, coefficient in trace.items())
            ),
            "reduced_burau_matrix_sha256": hashlib.sha256(matrix_json.encode()).hexdigest(),
        }
        _, node_id, u_upper, _, _ = representation_maps[identity]
        if u_upper is not None:
            values["snapshot_u_upper"] = str(u_upper)
        if node_id is not None:
            kind, action = policy_stops[int(node_id)]
            values["q254_stop_kind"] = kind
            if action is not None:
                values["q254_preferred_cc_u63"] = str(action)
        for fingerprint_id, value in values.items():
            value_type = definitions[fingerprint_id][0]
            rows.append(
                (
                    identity,
                    fingerprint_id,
                    value_type,
                    value,
                    value_hash(fingerprint_id, value_type, value),
                )
            )
        if index % 1000 == 0:
            print(f"fingerprinted {index}/{len(representations)}", flush=True)

    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    output = sqlite3.connect(temp)
    output.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE fingerprint_definitions(
            fingerprint_key INTEGER PRIMARY KEY,
            fingerprint_id TEXT NOT NULL UNIQUE,
            value_type TEXT NOT NULL,
            scope TEXT NOT NULL,
            valid_under TEXT NOT NULL,
            safe_use TEXT NOT NULL,
            definition TEXT NOT NULL
        );
        CREATE TABLE representation_map(
            representation_id TEXT PRIMARY KEY,
            stopping_key BLOB,
            graph_node_id INTEGER,
            knot_id TEXT,
            identification_class TEXT
        ) WITHOUT ROWID;
        CREATE TABLE fingerprint_values(
            value_key INTEGER PRIMARY KEY,
            fingerprint_key INTEGER NOT NULL,
            value_type TEXT NOT NULL,
            value_text TEXT NOT NULL,
            value_sha256 BLOB NOT NULL CHECK(length(value_sha256)=32),
            UNIQUE(fingerprint_key,value_text)
        );
        CREATE TABLE representation_fingerprints(
            representation_id TEXT NOT NULL,
            value_key INTEGER NOT NULL,
            PRIMARY KEY(representation_id,value_key)
        ) WITHOUT ROWID;
        CREATE INDEX fingerprints_reverse
            ON representation_fingerprints(value_key,representation_id);
        CREATE VIEW fingerprint_representation_map AS
            SELECT d.fingerprint_id,v.value_type,v.value_text,v.value_sha256,
                   p.representation_id,m.knot_id,m.identification_class,
                   m.stopping_key,m.graph_node_id
            FROM representation_fingerprints p
            JOIN fingerprint_values v USING(value_key)
            JOIN fingerprint_definitions d USING(fingerprint_key)
            JOIN representation_map m USING(representation_id);
        """
    )
    metadata = {
        "schema": "unknotdb-braid-fingerprints-v2",
        "lookup_maps_sha256": file_sha256(args.maps),
        "identification_maps_sha256": file_sha256(args.identifications),
        "graph_snapshot_sha256": graph_sha256,
        "rf_invariants_source": str((args.rf_src / "rf_knots/invariants.py").resolve()),
        "rf_invariants_sha256": file_sha256(args.rf_src / "rf_knots/invariants.py"),
        "proof_status": "search-metadata-never-a-knot-proof",
    }
    output.executemany("INSERT INTO meta VALUES (?,?)", sorted(metadata.items()))
    output.executemany(
        "INSERT INTO fingerprint_definitions VALUES (?,?,?,?,?,?,?)",
        [
            (fingerprint_key, fingerprint_id, *definition)
            for fingerprint_key, (fingerprint_id, definition) in enumerate(
                sorted(definitions.items()), 1
            )
        ],
    )
    output.executemany(
        "INSERT INTO representation_map VALUES (?,?,?,?,?)",
        [
            (identity, key, node_id, knot_id, evidence_class)
            for identity, (
                key,
                node_id,
                _,
                knot_id,
                evidence_class,
            ) in representation_maps.items()
        ],
    )
    fingerprint_keys = {
        fingerprint_id: fingerprint_key
        for fingerprint_key, fingerprint_id in output.execute(
            "SELECT fingerprint_key,fingerprint_id FROM fingerprint_definitions"
        )
    }
    unique_values = sorted(
        {(fingerprint_keys[fingerprint_id], value_type, value, digest) for _, fingerprint_id, value_type, value, digest in rows},
        key=lambda item: (item[0], item[2]),
    )
    output.executemany(
        "INSERT INTO fingerprint_values VALUES (?,?,?,?,?)",
        [
            (value_key, fingerprint_key, value_type, value, digest)
            for value_key, (fingerprint_key, value_type, value, digest) in enumerate(
                unique_values, 1
            )
        ],
    )
    value_keys = {
        (fingerprint_key, value): value_key
        for value_key, fingerprint_key, value in output.execute(
            "SELECT value_key,fingerprint_key,value_text FROM fingerprint_values"
        )
    }
    output.executemany(
        "INSERT INTO representation_fingerprints VALUES (?,?)",
        [
            (identity, value_keys[(fingerprint_keys[fingerprint_id], value)])
            for identity, fingerprint_id, _, value, _ in rows
        ],
    )
    output.execute("PRAGMA optimize")
    integrity = output.execute("PRAGMA integrity_check").fetchone()[0]
    if integrity != "ok":
        raise RuntimeError(f"fingerprint sidecar integrity failure: {integrity}")
    output.commit()
    output.close()
    os.replace(temp, args.output)

    report = f"""# Braid and representation fingerprints v2

- Sidecar: `{args.output}` ({args.output.stat().st_size:,} bytes)
- Representations: {len(representations):,}
- Fingerprint definitions: {len(definitions):,}
- Stored postings: {len(rows):,}
- Graph snapshot SHA-256: `{graph_sha256}`

Every definition records `scope`, `valid_under`, and `safe_use`. None of these values is admitted as a general knot invariant. Reverse lookup intersects the indexed postings `(fingerprint_id,value_text)` and is intended for candidate generation, bucketing and ranking only.
"""
    args.report.write_text(report)
    print(
        f"representations={len(representations)} definitions={len(definitions)} "
        f"postings={len(rows)} bytes={args.output.stat().st_size}"
    )


if __name__ == "__main__":
    main()
