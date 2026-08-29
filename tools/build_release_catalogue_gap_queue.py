#!/usr/bin/env python3
"""Build a hash-pinned U-witness queue from one coherent release bundle.

The queue is metadata only.  Catalogue values never change proof-graph U;
only independently replayed proof edges may do that.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import sqlite3
from pathlib import Path

EXACT_RE = re.compile(r"^\s*(\d+)\s*$")
RANGE_RE = re.compile(r"^\s*\[\s*(\d+)\s*,\s*(\d+)\s*\]\s*$")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def parse_u(raw: str) -> tuple[int, int, str] | None:
    if match := EXACT_RE.fullmatch(raw):
        value = int(match.group(1))
        return value, value, "exact"
    if match := RANGE_RE.fullmatch(raw):
        lower, upper = map(int, match.groups())
        if lower <= upper:
            return lower, upper, "range"
    return None


def read_meta(path: Path) -> dict[str, str]:
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        return dict(connection.execute("SELECT key,value FROM meta"))
    finally:
        connection.close()


def atomic_write(path: Path, text: str) -> None:
    temporary = path.with_name(f"{path.name}.tmp-{os.getpid()}")
    temporary.write_text(text)
    os.replace(temporary, path)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--proof", type=Path, required=True)
    parser.add_argument("--identification", type=Path, required=True)
    parser.add_argument("--federation", type=Path, required=True)
    parser.add_argument("--queue", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--keys-output", type=Path)
    parser.add_argument("--keys-limit", type=int, default=0)
    parser.add_argument(
        "--allow-parent-sidecars",
        action="store_true",
        help=(
            "allow sidecars pinned to a parent proof when every identified key "
            "is present; suitable for selection artifacts, not release publication"
        ),
    )
    args = parser.parse_args()
    output_paths = [args.queue, args.manifest]
    if args.keys_output:
        output_paths.append(args.keys_output)
    for path in output_paths:
        if path.exists():
            raise FileExistsError(f"output already exists: {path}")
    if args.keys_limit < 0:
        raise ValueError("--keys-limit must be non-negative")

    hashes = {
        "proof_sha256": file_sha256(args.proof),
        "identification_sha256": file_sha256(args.identification),
        "federation_sha256": file_sha256(args.federation),
    }
    identification_meta = read_meta(args.identification)
    federation_meta = read_meta(args.federation)
    exact_proof_pin = (
        identification_meta.get("graph_snapshot_sha256") == hashes["proof_sha256"]
        and federation_meta.get("graph_snapshot_sha256") == hashes["proof_sha256"]
    )
    if not args.allow_parent_sidecars and not exact_proof_pin:
        raise ValueError("identification sidecar is not pinned to this proof graph")
    if (
        federation_meta.get("identification_sidecar_sha256")
        != hashes["identification_sha256"]
    ):
        raise ValueError(
            "federation sidecar is not pinned to this identification sidecar"
        )

    proof = sqlite3.connect(f"file:{args.proof}?mode=ro", uri=True)
    identity = sqlite3.connect(f"file:{args.identification}?mode=ro", uri=True)
    federation = sqlite3.connect(f"file:{args.federation}?mode=ro", uri=True)
    try:
        nodes = {
            bytes(rep_key): (int(node_id), int(acs10), int(graph_u))
            for rep_key, node_id, acs10, graph_u in proof.execute(
                """
                SELECT k.rep_key,n.node_id,n.acs10,n.u_upper_bound
                FROM node_keys k JOIN nodes n USING(node_id)
                WHERE n.acs10 IS NOT NULL AND n.u_upper_bound IS NOT NULL
                """
            )
        }
        claims: dict[str, tuple[int, int, str, str, str]] = {}
        rejected_claims = 0
        for canonical_id, raw, source_id, source_pointer in federation.execute(
            """
            SELECT canonical_id,value_text,source_id,source_pointer
            FROM knot_property_map
            WHERE property_name='unknotting_number'
            """
        ):
            parsed = parse_u(str(raw))
            if parsed is None:
                rejected_claims += 1
                continue
            lower, upper, kind = parsed
            row = (lower, upper, kind, str(source_id), str(source_pointer))
            previous = claims.setdefault(str(canonical_id), row)
            if previous != row:
                raise ValueError(f"conflicting U claims for {canonical_id}")

        rows = []
        mapped = 0
        with_claim = 0
        for rep_key, knot_id, evidence_class, mapping_status in identity.execute(
            """
            SELECT rep_key,knot_id,evidence_class,mapping_status
            FROM graph_vertex_knot_map
            """
        ):
            mapped += 1
            key = bytes(rep_key)
            graph = nodes.get(key)
            if graph is None:
                raise ValueError(f"identified key absent from proof graph: {key.hex()}")
            catalogue_id = str(knot_id).removeprefix("knot:")
            claim = claims.get(catalogue_id)
            if claim is None:
                continue
            with_claim += 1
            lower, upper, kind, source_id, source_pointer = claim
            node_id, acs10, graph_u = graph
            if graph_u <= upper:
                continue
            rows.append(
                (
                    node_id,
                    str(knot_id),
                    str(evidence_class),
                    str(mapping_status),
                    key.hex().upper(),
                    acs10,
                    graph_u,
                    lower,
                    upper,
                    kind,
                    graph_u - upper,
                    source_id,
                    source_pointer,
                )
            )
    finally:
        proof.close()
        identity.close()
        federation.close()

    rows.sort(key=lambda row: (row[5], -row[10], row[4], row[1]))
    header = (
        "rank",
        "node_id",
        "knot_id",
        "identity_evidence",
        "mapping_status",
        "rep_key",
        "acs10",
        "graph_u_upper",
        "catalogue_u_lower",
        "catalogue_u_upper",
        "catalogue_claim_kind",
        "gap_to_catalogue_upper",
        "catalogue_source_id",
        "catalogue_source_pointer",
    )
    temporary = args.queue.with_name(f"{args.queue.name}.tmp-{os.getpid()}")
    with temporary.open("w", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(header)
        for rank, row in enumerate(rows, 1):
            writer.writerow((rank, *row))
    os.replace(temporary, args.queue)

    selected_keys = rows[: args.keys_limit or len(rows)] if args.keys_output else []
    if args.keys_output:
        atomic_write(
            args.keys_output,
            "".join(f"{row[4]}\n" for row in selected_keys),
        )

    manifest = {
        "format": "unknotdb-release-catalogue-gap-queue-v0",
        "proof": str(args.proof),
        "identification": str(args.identification),
        "federation": str(args.federation),
        **hashes,
        "queue": str(args.queue),
        "queue_sha256": file_sha256(args.queue),
        "keys_output": str(args.keys_output) if args.keys_output else None,
        "keys_output_sha256": file_sha256(args.keys_output)
        if args.keys_output
        else None,
        "keys_count": len(selected_keys),
        "order": ["acs10 ascending", "gap descending", "rep_key", "knot_id"],
        "sidecar_compatibility": (
            "exact-proof-hash-pin"
            if exact_proof_pin
            else "parent-sidecars-all-identified-keys-present-selection-only"
        ),
        "mapped_graph_vertices": mapped,
        "mapped_vertices_with_parseable_catalogue_u": with_claim,
        "parseable_catalogue_claims": len(claims),
        "rejected_catalogue_claims": rejected_claims,
        "actionable_rows": len(rows),
        "first_actionable": dict(zip(header[1:], rows[0], strict=True))
        if rows
        else None,
        "proof_semantics": "metadata-only; catalogue claims never change graph U",
    }
    atomic_write(args.manifest, json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(json.dumps(manifest, sort_keys=True))


if __name__ == "__main__":
    main()
