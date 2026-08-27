#!/usr/bin/env python3
"""Import provenance-bearing catalogue U claims and rank graph discrepancies.

The generated SQLite database is a metadata sidecar.  Catalogue claims never
change proof-graph distances and never create proof edges.  The TSV queue ranks
only actionable cases where a catalogue upper endpoint is strictly below the
graph's replay-validated U upper bound.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import os
import re
import shutil
import sqlite3
import subprocess
import tempfile
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


def convert_xls_to_csv(source: Path, soffice: str, destination: Path) -> None:
    executable = shutil.which(soffice) or (soffice if Path(soffice).exists() else None)
    if executable is None:
        raise FileNotFoundError(f"LibreOffice executable not found: {soffice}")
    subprocess.run(
        [
            executable,
            "--headless",
            "--convert-to",
            "csv",
            "--outdir",
            str(destination),
            str(source),
        ],
        check=True,
        capture_output=True,
        text=True,
    )


def load_claims(
    source: Path, soffice: str
) -> tuple[dict[str, tuple[int, int, str, str]], int]:
    with tempfile.TemporaryDirectory(prefix="unknotdb-knotinfo-") as temporary:
        directory = Path(temporary)
        convert_xls_to_csv(source, soffice, directory)
        csv_path = directory / f"{source.stem}.csv"
        if not csv_path.exists():
            candidates = list(directory.glob("*.csv"))
            if len(candidates) != 1:
                raise RuntimeError("LibreOffice did not produce one unambiguous CSV")
            csv_path = candidates[0]
        claims: dict[str, tuple[int, int, str, str]] = {}
        rejected = 0
        with csv_path.open(newline="", encoding="utf-8-sig") as stream:
            reader = csv.DictReader(stream)
            required = {"name", "unknotting_number"}
            if not required.issubset(reader.fieldnames or ()):
                raise ValueError("catalogue CSV lacks name/unknotting_number columns")
            for row in reader:
                name = row["name"].strip()
                raw = row["unknotting_number"].strip()
                if not name or name == "Name" or not raw:
                    continue
                parsed = parse_u(raw)
                if parsed is None:
                    rejected += 1
                    continue
                previous = claims.setdefault(name, (*parsed, raw))
                if previous != (*parsed, raw):
                    raise ValueError(f"conflicting catalogue U rows for {name}")
        return claims, rejected


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--identification-sidecar", type=Path, required=True)
    parser.add_argument("--catalogue-xls", type=Path, required=True)
    parser.add_argument("--source-id", required=True)
    parser.add_argument("--source-url", required=True)
    parser.add_argument("--retrieved-at", required=True)
    parser.add_argument("--soffice", default="soffice")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--queue", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    for path in (args.output, args.queue, args.report):
        if path.exists():
            raise FileExistsError(f"output already exists: {path}")

    claims, rejected = load_claims(args.catalogue_xls, args.soffice)
    snapshot_sha256 = file_sha256(args.snapshot)
    identification_sha256 = file_sha256(args.identification_sidecar)
    catalogue_sha256 = file_sha256(args.catalogue_xls)

    graph = sqlite3.connect(f"file:{args.snapshot}?mode=ro", uri=True)
    graph_nodes = {
        int(node_id): (int(acs10), int(u_upper), bytes(rep_key))
        for node_id, acs10, u_upper, rep_key in graph.execute(
            """
            SELECT n.node_id,n.acs10,n.u_upper_bound,k.rep_key
            FROM nodes n JOIN node_keys k USING(node_id)
            WHERE n.acs10 IS NOT NULL AND n.u_upper_bound IS NOT NULL
            """
        )
    }
    graph_nodes_by_key = {
        rep_key: (node_id, acs10, u_upper)
        for node_id, (acs10, u_upper, rep_key) in graph_nodes.items()
    }
    if len(graph_nodes_by_key) != len(graph_nodes):
        raise ValueError("graph contains duplicate canonical representation keys")
    graph.close()

    identity = sqlite3.connect(f"file:{args.identification_sidecar}?mode=ro", uri=True)
    mappings = list(
        identity.execute(
            """
            SELECT m.representation_id,m.knot_id,m.evidence_class,
                   r.graph_node_id,r.stopping_key
            FROM effective_representation_knot_map m
            JOIN representations r USING(representation_id)
            WHERE r.graph_node_id IS NOT NULL AND r.graph_u_upper IS NOT NULL
            """
        )
    )
    identity.close()

    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    connection = sqlite3.connect(temporary)
    connection.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE catalogue_sources(
            source_id TEXT PRIMARY KEY,
            source_url TEXT NOT NULL,
            retrieved_at TEXT NOT NULL,
            source_sha256 TEXT NOT NULL,
            source_format TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE catalogue_u_claims(
            knot_id TEXT NOT NULL,
            source_id TEXT NOT NULL,
            raw_value TEXT NOT NULL,
            u_lower INTEGER NOT NULL CHECK(u_lower >= 0),
            u_upper INTEGER NOT NULL CHECK(u_upper >= u_lower),
            claim_kind TEXT NOT NULL CHECK(claim_kind IN ('exact','range')),
            PRIMARY KEY(knot_id,source_id),
            FOREIGN KEY(source_id) REFERENCES catalogue_sources(source_id)
        ) WITHOUT ROWID;
        CREATE INDEX catalogue_u_reverse ON catalogue_u_claims(u_upper,u_lower,knot_id);
        CREATE TABLE graph_catalogue_comparison(
            node_id INTEGER NOT NULL,
            knot_id TEXT NOT NULL,
            representation_id TEXT NOT NULL,
            identity_evidence TEXT NOT NULL,
            rep_key BLOB NOT NULL CHECK(length(rep_key)=32),
            acs10 INTEGER NOT NULL,
            graph_u_upper INTEGER NOT NULL,
            catalogue_u_lower INTEGER NOT NULL,
            catalogue_u_upper INTEGER NOT NULL,
            catalogue_claim_kind TEXT NOT NULL,
            disposition TEXT NOT NULL,
            gap_to_catalogue_upper INTEGER NOT NULL,
            PRIMARY KEY(node_id,knot_id)
        ) WITHOUT ROWID;
        CREATE INDEX comparison_queue ON graph_catalogue_comparison(
            disposition,acs10,node_id,knot_id
        );
        """
    )
    meta = {
        "schema": "unknotdb-catalogue-u-sidecar-v0",
        "proof_status": "metadata-only-cannot-change-proof-graph",
        "snapshot_sha256": snapshot_sha256,
        "identification_sidecar_sha256": identification_sha256,
        "catalogue_sha256": catalogue_sha256,
        "queue_order": "acs10,node_id,knot_id",
    }
    connection.executemany("INSERT INTO meta VALUES (?,?)", sorted(meta.items()))
    connection.execute(
        "INSERT INTO catalogue_sources VALUES (?,?,?,?,?)",
        (
            args.source_id,
            args.source_url,
            args.retrieved_at,
            catalogue_sha256,
            "KnotInfo XLS",
        ),
    )
    connection.executemany(
        "INSERT INTO catalogue_u_claims VALUES (?,?,?,?,?,?)",
        [
            (f"knot:{name}", args.source_id, raw, lower, upper, kind)
            for name, (lower, upper, kind, raw) in sorted(claims.items())
        ],
    )

    # A graph node can have several RF representations of one named knot. Keep
    # the strongest identity tier and lexicographically first representation.
    comparison: dict[tuple[int, str], tuple] = {}
    evidence_rank = {"verified": 0, "attested": 1}
    for representation_id, knot_id, evidence, node_id, stopping_key in mappings:
        # Node IDs are snapshot-local and can be reassigned when a new immutable
        # snapshot is published.  The canonical stopping key is the durable join.
        # Retain the old node ID only as a compatibility fallback for legacy rows
        # that predate stopping-key storage.
        if stopping_key is not None:
            durable_key = bytes(stopping_key)
            current = graph_nodes_by_key.get(durable_key)
            if current is None:
                continue
            node_id, acs10, graph_u = current
            rep_key = durable_key
        else:
            node_id = int(node_id)
            graph_row = graph_nodes.get(node_id)
            if graph_row is None:
                continue
            acs10, graph_u, rep_key = graph_row
        name = str(knot_id).removeprefix("knot:")
        claim = claims.get(name)
        if claim is None:
            continue
        lower, upper, kind, _ = claim
        if graph_u > upper:
            disposition = "needs-shorter-witness"
        elif graph_u < lower:
            disposition = "graph-stronger-than-catalogue"
        elif graph_u == upper:
            disposition = "matches-catalogue-upper"
        else:
            disposition = "inside-catalogue-range"
        row = (
            node_id,
            knot_id,
            representation_id,
            evidence,
            rep_key,
            acs10,
            graph_u,
            lower,
            upper,
            kind,
            disposition,
            graph_u - upper,
        )
        key = (node_id, knot_id)
        previous = comparison.get(key)
        if previous is None or (evidence_rank[evidence], representation_id) < (
            evidence_rank[previous[3]],
            previous[2],
        ):
            comparison[key] = row
    connection.executemany(
        "INSERT INTO graph_catalogue_comparison VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        comparison.values(),
    )
    connection.execute("PRAGMA optimize")
    if connection.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
        raise RuntimeError("catalogue sidecar integrity failure")
    connection.commit()

    queue_rows = list(
        connection.execute(
            """
            SELECT node_id,knot_id,representation_id,identity_evidence,hex(rep_key),
                   acs10,graph_u_upper,catalogue_u_lower,catalogue_u_upper,
                   catalogue_claim_kind,gap_to_catalogue_upper
            FROM graph_catalogue_comparison
            WHERE disposition='needs-shorter-witness'
            ORDER BY acs10,node_id,knot_id
            """
        )
    )
    counts = dict(
        connection.execute(
            "SELECT disposition,count(*) FROM graph_catalogue_comparison GROUP BY disposition"
        )
    )
    connection.close()
    os.replace(temporary, args.output)

    queue_temporary = args.queue.with_name(f"{args.queue.name}.tmp-{os.getpid()}")
    with queue_temporary.open("w", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(
            (
                "rank",
                "node_id",
                "knot_id",
                "representation_id",
                "identity_evidence",
                "rep_key",
                "acs10",
                "graph_u_upper",
                "catalogue_u_lower",
                "catalogue_u_upper",
                "catalogue_claim_kind",
                "gap_to_catalogue_upper",
            )
        )
        for rank, row in enumerate(queue_rows, 1):
            writer.writerow((rank, *row))
    os.replace(queue_temporary, args.queue)

    first = queue_rows[0] if queue_rows else None
    report = f"""# Catalogue U import and ASC10 discrepancy queue

- Graph snapshot: `{args.snapshot}`
- Graph SHA-256: `{snapshot_sha256}`
- Identification sidecar: `{args.identification_sidecar}`
- Identification SHA-256: `{identification_sha256}`
- Catalogue: `{args.source_id}` from `{args.source_url}`, retrieved `{args.retrieved_at}`
- Catalogue source SHA-256: `{catalogue_sha256}`
- Parsed catalogue claims: {len(claims):,}; rejected nonempty values: {rejected:,}
- Graph/name comparisons: {len(comparison):,}
- Needs a shorter witness: {counts.get("needs-shorter-witness", 0):,}
- Matches catalogue upper endpoint: {counts.get("matches-catalogue-upper", 0):,}
- Inside catalogue interval: {counts.get("inside-catalogue-range", 0):,}
- Graph stronger than catalogue lower endpoint: {counts.get("graph-stronger-than-catalogue", 0):,}
- Sidecar: `{args.output}`
- Queue: `{args.queue}`

Catalogue claims are metadata only. They never alter `U_upper`, route pointers,
proof programs, or graph edges. Queue order is exactly `(ASC10, node_id, knot_id)`.
"""
    if first:
        report += (
            f"\nFirst actionable row: node `{first[0]}`, `{first[1]}`, "
            f"ASC10 `{first[5]}`, graph U upper `{first[6]}`, "
            f"catalogue `[{first[7]},{first[8]}]`.\n"
        )
    report_temporary = args.report.with_name(f"{args.report.name}.tmp-{os.getpid()}")
    report_temporary.write_text(report)
    os.replace(report_temporary, args.report)
    print(
        f"claims={len(claims)} comparisons={len(comparison)} "
        f"actionable={len(queue_rows)} first={first[:9] if first else None}"
    )


if __name__ == "__main__":
    main()
