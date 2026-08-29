#!/usr/bin/env python3
"""Mine reusable anchored proof-template families from a schema-v3 snapshot.

This is derived analytics only: it never changes proof programs or graph rows.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import os
import re
import sqlite3
from collections import Counter
from pathlib import Path

OP_RE = re.compile(
    r"Action\((Reduce|Commute|Braid|Insert|Destabilize|StabilizePositive|"
    r"StabilizeNegative|StabilizeAt|CrossingChange|DescendingCollapse|"
    r"PlanarCertificateCollapse)|"
    r"(NormalizeOrigin|RotateOriginLeft|MirrorOrbit|Checkpoint)"
)


def write_tsv(path: Path, header: list[str], rows: list[tuple[object, ...]]) -> None:
    with path.open("w", newline="") as stream:
        writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
        writer.writerow(header)
        writer.writerows(rows)


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--templates", type=Path, required=True)
    parser.add_argument("--prefix", type=Path, required=True)
    parser.add_argument("--routing-sidecar", type=Path)
    parser.add_argument("--sidecar", type=Path)
    args = parser.parse_args()

    templates: dict[int, dict[str, object]] = {}
    with args.templates.open(newline="") as stream:
        for row in csv.DictReader(stream, delimiter="\t"):
            template_id = int(row["template_id"])
            ops = tuple(a or b for a, b in OP_RE.findall(row["template"]))
            family = " > ".join(ops) if ops else "coordinate-only"
            templates[template_id] = {
                "uses": int(row["uses"]),
                "cc_cost": int(row["cc_cost"]),
                "instructions": int(row["instructions"]),
                "sha256": row["sha256"],
                "ops": ops,
                "family": family,
            }

    connection = sqlite3.connect(f"file:{args.snapshot}?mode=ro", uri=True)
    snapshot_sha256 = file_sha256(args.snapshot)
    routing_sha256 = None
    if args.routing_sidecar:
        routing = sqlite3.connect(f"file:{args.routing_sidecar}?mode=ro", uri=True)
        pinned = routing.execute(
            "SELECT value FROM meta WHERE key='proof_sha256'"
        ).fetchone()[0]
        routing.close()
        if pinned != snapshot_sha256:
            raise ValueError("routing sidecar is not pinned to the template snapshot")
        routing_sha256 = file_sha256(args.routing_sidecar)
        connection.execute("ATTACH DATABASE ? AS routing", (str(args.routing_sidecar),))
        utility_sql = """
            SELECT e.program_id,
                   count(*),
                   sum(CASE WHEN n.next_unknot_edge=e.edge_id THEN 1 ELSE 0 END),
                   sum(CASE WHEN n.next_acs10_edge=e.edge_id THEN 1 ELSE 0 END),
                   sum(CASE WHEN r.next_shortest_edge=e.edge_id THEN 1 ELSE 0 END),
                   sum(CASE WHEN r.next_l1000_edge=e.edge_id THEN 1 ELSE 0 END),
                   min(e.cc_cost+t.u_upper_bound-n.u_upper_bound),
                   max(e.cc_cost+t.u_upper_bound-n.u_upper_bound)
            FROM edges e
            JOIN nodes n ON n.node_id=e.source_node
            JOIN nodes t ON t.node_id=e.target_node
            JOIN routing.routes r ON r.node_id=e.source_node
            GROUP BY e.program_id
        """
    else:
        utility_sql = """
            SELECT e.program_id,
                   count(*),
                   sum(CASE WHEN n.next_unknot_edge=e.edge_id THEN 1 ELSE 0 END),
                   sum(CASE WHEN n.next_acs10_edge=e.edge_id THEN 1 ELSE 0 END),
                   0,0,
                   min(e.cc_cost+t.u_upper_bound-n.u_upper_bound),
                   max(e.cc_cost+t.u_upper_bound-n.u_upper_bound)
            FROM edges e
            JOIN nodes n ON n.node_id=e.source_node
            JOIN nodes t ON t.node_id=e.target_node
            GROUP BY e.program_id
        """
    utility = {
        int(row[0]): tuple(int(value) for value in row[1:])
        for row in connection.execute(utility_sql)
    }
    anchors = list(
        connection.execute(
            """
            SELECT program_id,program_anchor_x,program_anchor_y,count(*)
            FROM edges
            GROUP BY program_id,program_anchor_x,program_anchor_y
            ORDER BY program_id,program_anchor_x,program_anchor_y
            """
        )
    )
    reverse_pairs = [
        tuple(int(value) for value in row)
        for row in connection.execute(
            """
            SELECT a.program_id,b.program_id,count(*)
            FROM edges a JOIN edges b
              ON b.source_node=a.target_node AND b.target_node=a.source_node
            WHERE a.edge_id < b.edge_id
            GROUP BY a.program_id,b.program_id
            ORDER BY count(*) DESC,a.program_id,b.program_id
            """
        )
    ]
    connection.close()

    family_uses: Counter[str] = Counter()
    family_templates: Counter[str] = Counter()
    family_selected_u: Counter[str] = Counter()
    family_selected_acs: Counter[str] = Counter()
    family_selected_shortest: Counter[str] = Counter()
    family_selected_l1000: Counter[str] = Counter()
    subsequences: Counter[tuple[str, ...]] = Counter()
    for template_id, record in templates.items():
        family = str(record["family"])
        uses = int(record["uses"])
        family_uses[family] += uses
        family_templates[family] += 1
        (
            _,
            selected_u,
            selected_acs,
            selected_shortest,
            selected_l1000,
            _,
            _,
        ) = utility.get(template_id, (0, 0, 0, 0, 0, 0, 0))
        family_selected_u[family] += selected_u
        family_selected_acs[family] += selected_acs
        family_selected_shortest[family] += selected_shortest
        family_selected_l1000[family] += selected_l1000
        ops = tuple(record["ops"])
        for length in range(1, min(4, len(ops)) + 1):
            for start in range(len(ops) - length + 1):
                subsequences[ops[start : start + length]] += uses

    prefix = args.prefix
    family_path = prefix.with_name(prefix.name + "-families.tsv")
    subsequence_path = prefix.with_name(prefix.name + "-subsequences.tsv")
    reverse_path = prefix.with_name(prefix.name + "-reverse-pairs.tsv")
    utility_path = prefix.with_name(prefix.name + "-utility.tsv")
    report_path = prefix.with_name(prefix.name + ".md")

    family_rows = sorted(
        (
            uses,
            family_templates[family],
            family_selected_u[family],
            family_selected_acs[family],
            family_selected_shortest[family],
            family_selected_l1000[family],
            family,
        )
        for family, uses in family_uses.items()
    )
    family_rows.reverse()
    write_tsv(
        family_path,
        [
            "edge_uses",
            "templates",
            "selected_u",
            "selected_acs10",
            "selected_shortest_u",
            "selected_l1000",
            "semantic_family",
        ],
        family_rows,
    )
    write_tsv(
        subsequence_path,
        ["weighted_occurrences", "length", "semantic_subsequence"],
        sorted(
            (
                count,
                len(sequence),
                " > ".join(sequence),
            )
            for sequence, count in subsequences.items()
        )[::-1],
    )
    write_tsv(
        reverse_path,
        ["forward_template", "reverse_template", "verified_endpoint_pairs"],
        reverse_pairs,
    )
    utility_rows = []
    for template_id, record in templates.items():
        (
            edge_uses,
            selected_u,
            selected_acs,
            selected_shortest,
            selected_l1000,
            min_slack,
            max_slack,
        ) = utility.get(template_id, (0, 0, 0, 0, 0, 0, 0))
        utility_rows.append(
            (
                template_id,
                edge_uses,
                selected_u,
                selected_acs,
                selected_shortest,
                selected_l1000,
                min_slack,
                max_slack,
                record["cc_cost"],
                record["instructions"],
                record["family"],
                record["sha256"],
            )
        )
    utility_rows.sort(key=lambda row: (-row[2], -row[1], row[0]))
    write_tsv(
        utility_path,
        [
            "template_id",
            "edge_uses",
            "selected_u",
            "selected_acs10",
            "selected_shortest_u",
            "selected_l1000",
            "min_bellman_slack",
            "max_bellman_slack",
            "cc_cost",
            "instructions",
            "semantic_family",
            "sha256",
        ],
        utility_rows,
    )

    paired_edges = sum(row[2] for row in reverse_pairs)
    report = f"""# Anchored program-template analysis

- Snapshot: `{args.snapshot}`
- Exact anchored templates: {len(templates):,}
- Semantic opcode families: {len(family_rows):,}
- Weighted contiguous subsequences (length 1–4): {len(subsequences):,}
- Independently verified reverse-endpoint edge pairs: {paired_edges:,}

Families intentionally ignore numeric coordinates while retaining semantic operation order. They are search indexes, not new proof assertions. A reverse pair means that independently replayed stored edges connect the same two canonical vertices in opposite directions; it does not assert that their byte programs are formal syntactic inverses.

## Leading families

| Edge uses | Templates | Selected U | Family |
|---:|---:|---:|---|
"""
    for uses, count, selected_u, _, _, _, family in family_rows[:20]:
        report += f"| {uses:,} | {count:,} | {selected_u:,} | `{family}` |\n"
    report += "\n## Artifacts\n\n"
    for path in (family_path, subsequence_path, reverse_path, utility_path):
        report += f"- `{path}`\n"
    report_path.write_text(report)

    if args.sidecar:
        if args.sidecar.exists():
            raise FileExistsError(f"pattern sidecar already exists: {args.sidecar}")
        temporary = args.sidecar.with_name(f".{args.sidecar.name}.{os.getpid()}.part")
        sidecar = sqlite3.connect(temporary)
        sidecar.executescript(
            """
            PRAGMA journal_mode=OFF;
            PRAGMA synchronous=OFF;
            PRAGMA user_version=1;
            CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
            CREATE TABLE templates(
                template_id INTEGER PRIMARY KEY,
                sha256 TEXT NOT NULL UNIQUE,
                semantic_family TEXT NOT NULL,
                cc_cost INTEGER NOT NULL,
                instructions INTEGER NOT NULL,
                edge_uses INTEGER NOT NULL,
                selected_u INTEGER NOT NULL,
                selected_acs10 INTEGER NOT NULL,
                selected_shortest_u INTEGER NOT NULL,
                selected_l1000 INTEGER NOT NULL,
                min_bellman_slack INTEGER NOT NULL,
                max_bellman_slack INTEGER NOT NULL
            );
            CREATE INDEX templates_by_family ON templates(semantic_family,edge_uses);
            CREATE TABLE anchor_bindings(
                template_id INTEGER NOT NULL,
                anchor_x INTEGER NOT NULL,
                anchor_y INTEGER NOT NULL,
                edge_uses INTEGER NOT NULL,
                PRIMARY KEY(template_id,anchor_x,anchor_y)
            ) WITHOUT ROWID;
            CREATE TABLE subsequences(
                semantic_subsequence TEXT PRIMARY KEY,
                instruction_length INTEGER NOT NULL,
                weighted_occurrences INTEGER NOT NULL
            ) WITHOUT ROWID;
            CREATE INDEX subsequences_by_use ON subsequences(weighted_occurrences,semantic_subsequence);
            CREATE TABLE reverse_pairs(
                forward_template INTEGER NOT NULL,
                reverse_template INTEGER NOT NULL,
                verified_endpoint_pairs INTEGER NOT NULL,
                PRIMARY KEY(forward_template,reverse_template)
            ) WITHOUT ROWID;
            """
        )
        sidecar.executemany(
            "INSERT INTO meta VALUES (?,?)",
            sorted(
                {
                    "schema": "unknotdb-program-pattern-sidecar-v1",
                    "proof_sha256": snapshot_sha256,
                    "routing_sidecar_sha256": routing_sha256 or "none",
                    "templates_report_sha256": file_sha256(args.templates),
                    "proof_semantics": "derived indexes only; no new proof assertions",
                }.items()
            ),
        )
        sidecar_template_rows = [
            (
                row[0],
                row[11],
                row[10],
                row[8],
                row[9],
                row[1],
                row[2],
                row[3],
                row[4],
                row[5],
                row[6],
                row[7],
            )
            for row in utility_rows
        ]
        sidecar.executemany(
            "INSERT INTO templates VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            sidecar_template_rows,
        )
        sidecar.executemany("INSERT INTO anchor_bindings VALUES (?,?,?,?)", anchors)
        sidecar.executemany(
            "INSERT INTO subsequences VALUES (?,?,?)",
            [
                (" > ".join(sequence), len(sequence), count)
                for sequence, count in subsequences.items()
            ],
        )
        sidecar.executemany("INSERT INTO reverse_pairs VALUES (?,?,?)", reverse_pairs)
        sidecar.execute("PRAGMA optimize")
        if sidecar.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
            raise RuntimeError("program pattern sidecar integrity failure")
        sidecar.commit()
        sidecar.close()
        os.replace(temporary, args.sidecar)
        report += f"- `{args.sidecar}`\n"
        report_path.write_text(report)
    print(
        f"templates={len(templates)} families={len(family_rows)} "
        f"subsequences={len(subsequences)} reverse_endpoint_pairs={paired_edges}"
    )


if __name__ == "__main__":
    main()
