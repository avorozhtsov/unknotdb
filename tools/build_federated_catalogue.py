#!/usr/bin/env python3
"""Build a compact, provenance-bearing catalogue and CC-adjacency sidecar.

The sidecar is deliberately proof-external.  A catalogue value or adjacency
claim cannot change the replay-validated graph.  Only graph edges imported from
an immutable snapshot are labelled ``replay_verified``.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import shutil
import sqlite3
import subprocess
import tempfile
from collections.abc import Iterable
from pathlib import Path

SCHEMA = "unknotdb-federated-catalogue-v1"
KNOTINFO_NAME = re.compile(r"^(?:0_1|[3-9]_\d+|10_\d+|1[1-3][an]_\d+)$")

# Hot, searchable catalogue properties.  Large specialist polynomial tables
# remain in the hash-pinned source snapshot and can be added as another shard.
PROPERTY_FIELDS = (
    "category",
    "alternating",
    "crossing_number",
    "fibered",
    "unknotting_number",
    "three_genus",
    "crosscap_number",
    "bridge_index",
    "braid_index",
    "braid_length",
    "signature",
    "nakanishi_index",
    "arc_index",
    "tunnel_number",
    "morse_novikov_number",
    "alexander_polynomial",
    "alexander_polynomial_vector",
    "jones_polynomial",
    "jones_polynomial_vector",
    "conway_polynomial",
    "conway_polynomial_vector",
    "smooth_four_genus",
    "topological_four_genus",
    "determinant",
    "rasmussen_invariant",
    "ozsvath_szabo_tau_invariant",
    "volume",
    "arf_invariant",
    "turaev_genus",
    "positive_braid",
    "positive",
    "strongly_quasipositive",
    "quasipositive",
    "quasi_alternating",
    "almost_alternating",
    "adequate",
    "unknotting_number_algebraic",
    "homfly_polynomial",
    "homfly_polynomial_vector",
    "geometric_type",
)

REPRESENTATION_FIELDS = {
    "dt_notation": "dt",
    "gauss_notation": "gauss",
    "pd_notation": "pd",
    "braid_notation": "artin_braid_word",
}

IDENTIFIER_FIELDS = {
    "name": ("knotinfo", "canonical"),
    "dt_name": ("hoste-thistlethwaite", "alias"),
    "classical_conway_name": ("conway", "alias"),
}

INTEGER_FIELDS = {
    "category",
    "crossing_number",
    "three_genus",
    "crosscap_number",
    "bridge_index",
    "braid_index",
    "braid_length",
    "signature",
    "nakanishi_index",
    "arc_index",
    "tunnel_number",
    "morse_novikov_number",
    "determinant",
    "rasmussen_invariant",
    "ozsvath_szabo_tau_invariant",
    "arf_invariant",
    "turaev_genus",
    "unknotting_number_algebraic",
}


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def text_sha256(*parts: str) -> bytes:
    digest = hashlib.sha256()
    for part in parts:
        digest.update(part.encode())
        digest.update(b"\0")
    return digest.digest()


def convert_xls(source: Path, soffice: str, destination: Path) -> Path:
    executable = shutil.which(soffice) or (soffice if Path(soffice).exists() else None)
    if executable is None:
        raise FileNotFoundError(f"LibreOffice executable not found: {soffice}")
    subprocess.run(
        [
            str(executable),
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
    candidates = list(destination.glob("*.csv"))
    if len(candidates) != 1:
        raise RuntimeError("LibreOffice did not produce one unambiguous CSV")
    return candidates[0]


def load_knotinfo(
    source: Path, soffice: str, max_crossings: int
) -> list[dict[str, str]]:
    with tempfile.TemporaryDirectory(prefix="unknotdb-knotinfo-") as temporary:
        csv_path = convert_xls(source, soffice, Path(temporary))
        with csv_path.open(encoding="utf-8-sig", newline="") as stream:
            rows = list(csv.DictReader(stream))
    required = (
        set(PROPERTY_FIELDS) | set(REPRESENTATION_FIELDS) | set(IDENTIFIER_FIELDS)
    )
    if not rows or not required.issubset(rows[0]):
        missing = sorted(required - (set(rows[0]) if rows else set()))
        raise ValueError(f"KnotInfo snapshot lacks required columns: {missing}")
    result = []
    for row in rows:
        name = row["name"].strip()
        crossing = row["crossing_number"].strip()
        if name == "Name" or not name:
            continue
        if not KNOTINFO_NAME.fullmatch(name):
            raise ValueError(f"unexpected KnotInfo identifier: {name}")
        if not crossing.isdigit() or int(crossing) > max_crossings:
            continue
        result.append({key: value.strip() for key, value in row.items()})
    if len({row["name"] for row in result}) != len(result):
        raise ValueError("duplicate KnotInfo canonical name")
    return result


def value_type(field: str, value: str) -> str:
    if field in INTEGER_FIELDS and re.fullmatch(r"-?\d+", value):
        return "integer"
    if field == "unknotting_number":
        return "integer" if value.isdigit() else "integer_or_interval"
    if field in {"alternating", "fibered", "positive_braid", "positive", "ribbon"}:
        return "boolean_or_unknown"
    if field.endswith(("polynomial", "polynomial_vector")):
        return "polynomial"
    return "text"


def create_schema(db: sqlite3.Connection) -> None:
    db.executescript(
        """
        PRAGMA foreign_keys=ON;
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE catalogue_sources(
            source_id TEXT PRIMARY KEY,
            catalogue_kind TEXT NOT NULL,
            source_url TEXT NOT NULL,
            retrieved_at TEXT NOT NULL,
            source_sha256 TEXT NOT NULL CHECK(length(source_sha256)=64),
            source_format TEXT NOT NULL,
            source_scope TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE knots(
            knot_pk INTEGER PRIMARY KEY,
            canonical_id TEXT NOT NULL UNIQUE,
            crossing_number INTEGER NOT NULL CHECK(crossing_number>=0),
            table_kind TEXT NOT NULL
        );
        CREATE INDEX knots_by_crossing ON knots(crossing_number,canonical_id);
        CREATE TABLE knot_identifiers(
            scheme TEXT NOT NULL,
            identifier TEXT NOT NULL,
            knot_pk INTEGER NOT NULL,
            role TEXT NOT NULL CHECK(role IN ('canonical','alias')),
            source_id TEXT NOT NULL,
            source_pointer TEXT NOT NULL,
            PRIMARY KEY(scheme,identifier,knot_pk),
            FOREIGN KEY(knot_pk) REFERENCES knots(knot_pk),
            FOREIGN KEY(source_id) REFERENCES catalogue_sources(source_id)
        ) WITHOUT ROWID;
        CREATE INDEX identifiers_exact ON knot_identifiers(identifier,scheme,knot_pk);
        CREATE TABLE representations(
            representation_pk INTEGER PRIMARY KEY,
            knot_pk INTEGER NOT NULL,
            encoding TEXT NOT NULL,
            representation_text TEXT NOT NULL,
            representation_sha256 BLOB NOT NULL CHECK(length(representation_sha256)=32),
            source_id TEXT NOT NULL,
            source_pointer TEXT NOT NULL,
            preferred_rank INTEGER NOT NULL CHECK(preferred_rank>=0),
            UNIQUE(knot_pk,encoding,representation_sha256),
            FOREIGN KEY(knot_pk) REFERENCES knots(knot_pk),
            FOREIGN KEY(source_id) REFERENCES catalogue_sources(source_id)
        );
        CREATE INDEX representations_exact
            ON representations(encoding,representation_sha256,knot_pk);
        CREATE TABLE property_definitions(
            property_id INTEGER PRIMARY KEY,
            property_name TEXT NOT NULL UNIQUE,
            value_type TEXT NOT NULL,
            scope TEXT NOT NULL,
            source_column TEXT NOT NULL
        );
        CREATE TABLE property_values(
            value_id INTEGER PRIMARY KEY,
            property_id INTEGER NOT NULL,
            value_text TEXT NOT NULL,
            value_sha256 BLOB NOT NULL CHECK(length(value_sha256)=32),
            UNIQUE(property_id,value_sha256,value_text),
            FOREIGN KEY(property_id) REFERENCES property_definitions(property_id)
        );
        CREATE INDEX property_reverse ON property_values(property_id,value_text,value_id);
        CREATE TABLE knot_properties(
            knot_pk INTEGER NOT NULL,
            property_id INTEGER NOT NULL,
            value_id INTEGER NOT NULL,
            source_id TEXT NOT NULL,
            source_pointer TEXT NOT NULL,
            PRIMARY KEY(knot_pk,property_id,source_id),
            FOREIGN KEY(knot_pk) REFERENCES knots(knot_pk),
            FOREIGN KEY(property_id) REFERENCES property_definitions(property_id),
            FOREIGN KEY(value_id) REFERENCES property_values(value_id),
            FOREIGN KEY(source_id) REFERENCES catalogue_sources(source_id)
        ) WITHOUT ROWID;
        CREATE INDEX knot_properties_reverse ON knot_properties(property_id,value_id,knot_pk);
        CREATE TABLE graph_knot_links(
            knot_pk INTEGER NOT NULL,
            stopping_key BLOB NOT NULL CHECK(length(stopping_key)=32),
            graph_node_id INTEGER NOT NULL,
            graph_u_upper INTEGER,
            evidence_class TEXT NOT NULL CHECK(evidence_class IN ('verified','attested')),
            representation_id TEXT NOT NULL,
            PRIMARY KEY(knot_pk,stopping_key),
            FOREIGN KEY(knot_pk) REFERENCES knots(knot_pk)
        ) WITHOUT ROWID;
        CREATE INDEX graph_links_by_key ON graph_knot_links(stopping_key,knot_pk);
        CREATE TABLE adjacency_claims(
            claim_id BLOB PRIMARY KEY CHECK(length(claim_id)=32),
            source_id TEXT NOT NULL,
            source_knot_pk INTEGER NOT NULL,
            target_knot_pk INTEGER NOT NULL,
            relation TEXT NOT NULL CHECK(relation='crossing_change'),
            cc_cost INTEGER NOT NULL CHECK(cc_cost=1),
            status TEXT NOT NULL CHECK(status IN
                ('claimed','diagram_attested','replay_verified')),
            claim_scope TEXT NOT NULL CHECK(claim_scope IN
                ('knot_type','fixed_diagram','normalized_representation')),
            source_representation_pk INTEGER,
            target_representation_pk INTEGER,
            source_stopping_key BLOB CHECK(source_stopping_key IS NULL OR length(source_stopping_key)=32),
            target_stopping_key BLOB CHECK(target_stopping_key IS NULL OR length(target_stopping_key)=32),
            crossing_locator TEXT,
            witness_uri TEXT,
            witness_sha256 TEXT CHECK(witness_sha256 IS NULL OR length(witness_sha256)=64),
            graph_snapshot_sha256 TEXT CHECK(graph_snapshot_sha256 IS NULL OR length(graph_snapshot_sha256)=64),
            graph_edge_id INTEGER,
            source_pointer TEXT NOT NULL,
            CHECK(status!='diagram_attested' OR
                  (source_representation_pk IS NOT NULL AND crossing_locator IS NOT NULL)),
            CHECK(status!='replay_verified' OR
                  (source_stopping_key IS NOT NULL AND target_stopping_key IS NOT NULL AND
                   graph_snapshot_sha256 IS NOT NULL AND graph_edge_id IS NOT NULL)),
            FOREIGN KEY(source_id) REFERENCES catalogue_sources(source_id),
            FOREIGN KEY(source_knot_pk) REFERENCES knots(knot_pk),
            FOREIGN KEY(target_knot_pk) REFERENCES knots(knot_pk),
            FOREIGN KEY(source_representation_pk) REFERENCES representations(representation_pk),
            FOREIGN KEY(target_representation_pk) REFERENCES representations(representation_pk)
        ) WITHOUT ROWID;
        CREATE INDEX adjacency_out ON adjacency_claims(source_knot_pk,status,target_knot_pk);
        CREATE INDEX adjacency_in ON adjacency_claims(target_knot_pk,status,source_knot_pk);
        CREATE VIEW knot_property_map AS
            SELECT k.canonical_id,d.property_name,v.value_text,v.value_sha256,
                   p.source_id,p.source_pointer
            FROM knot_properties p JOIN knots k USING(knot_pk)
            JOIN property_definitions d USING(property_id)
            JOIN property_values v USING(value_id);
        CREATE VIEW adjacency_named AS
            SELECT lower(hex(a.claim_id)) AS claim_id,s.canonical_id AS source_knot,
                   t.canonical_id AS target_knot,a.status,a.claim_scope,
                   a.crossing_locator,a.witness_uri,a.graph_edge_id,a.source_id
            FROM adjacency_claims a
            JOIN knots s ON s.knot_pk=a.source_knot_pk
            JOIN knots t ON t.knot_pk=a.target_knot_pk;
        """
    )


def import_knotinfo(
    db: sqlite3.Connection, rows: list[dict[str, str]], source_id: str
) -> dict[str, int]:
    knot_pks: dict[str, int] = {}
    for knot_pk, row in enumerate(sorted(rows, key=lambda item: item["name"]), 1):
        name = row["name"]
        crossing = int(row["crossing_number"])
        kind = (
            "unknot"
            if crossing == 0
            else (
                "alternating"
                if "a_" in name
                else "nonalternating"
                if "n_" in name
                else "rolfsen"
            )
        )
        db.execute(
            "INSERT INTO knots VALUES (?,?,?,?)", (knot_pk, name, crossing, kind)
        )
        knot_pks[name] = knot_pk
        row_pointer = f"name={name}"
        for field, (scheme, role) in IDENTIFIER_FIELDS.items():
            identifier = row[field]
            if identifier:
                db.execute(
                    "INSERT OR IGNORE INTO knot_identifiers VALUES (?,?,?,?,?,?)",
                    (
                        scheme,
                        identifier,
                        knot_pk,
                        role,
                        source_id,
                        f"{row_pointer};column={field}",
                    ),
                )
        if re.fullmatch(r"1[0-3][an]_\d+", name):
            spherogram = "K" + name.replace("_", "")
            db.execute(
                "INSERT INTO knot_identifiers VALUES (?,?,?,?,?,?)",
                (
                    "spherogram",
                    spherogram,
                    knot_pk,
                    "alias",
                    source_id,
                    f"derived-from={name}",
                ),
            )
        for rank, (field, encoding) in enumerate(REPRESENTATION_FIELDS.items()):
            text = row[field]
            if text:
                db.execute(
                    "INSERT INTO representations(knot_pk,encoding,representation_text,representation_sha256,source_id,source_pointer,preferred_rank) VALUES (?,?,?,?,?,?,?)",
                    (
                        knot_pk,
                        encoding,
                        text,
                        text_sha256(encoding, text),
                        source_id,
                        f"{row_pointer};column={field}",
                        rank,
                    ),
                )

    property_ids = {field: index for index, field in enumerate(PROPERTY_FIELDS, 1)}
    for field, property_id in property_ids.items():
        examples = [row[field] for row in rows if row[field]]
        types = {value_type(field, value) for value in examples}
        db.execute(
            "INSERT INTO property_definitions VALUES (?,?,?,?,?)",
            (
                property_id,
                field,
                next(iter(types)) if len(types) == 1 else "text_or_interval",
                "knot_type_catalogue_claim",
                field,
            ),
        )
    value_cache: dict[tuple[int, str], int] = {}
    next_value_id = 1
    for row in sorted(rows, key=lambda item: item["name"]):
        knot_pk = knot_pks[row["name"]]
        for field, property_id in property_ids.items():
            value = row[field]
            if not value:
                continue
            cache_key = (property_id, value)
            value_id = value_cache.get(cache_key)
            if value_id is None:
                value_id = next_value_id
                next_value_id += 1
                value_cache[cache_key] = value_id
                db.execute(
                    "INSERT INTO property_values VALUES (?,?,?,?)",
                    (value_id, property_id, value, text_sha256(field, value)),
                )
            db.execute(
                "INSERT INTO knot_properties VALUES (?,?,?,?,?)",
                (
                    knot_pk,
                    property_id,
                    value_id,
                    source_id,
                    f"name={row['name']};column={field}",
                ),
            )
    return knot_pks


def import_graph_links(
    db: sqlite3.Connection,
    knot_pks: dict[str, int],
    graph_path: Path,
    identification_path: Path,
    graph_sha256: str,
) -> tuple[int, int, int]:
    identity = sqlite3.connect(f"file:{identification_path}?mode=ro", uri=True)
    mappings = list(
        identity.execute(
            """
            SELECT r.stopping_key,r.graph_node_id,r.graph_u_upper,m.knot_id,
                   m.evidence_class,m.representation_id
            FROM representations r JOIN effective_representation_knot_map m
              USING(representation_id)
            WHERE r.stopping_key IS NOT NULL
            ORDER BY m.evidence_class,m.representation_id
            """
        )
    )
    identity.close()
    evidence_rank = {"verified": 0, "attested": 1}
    links: dict[tuple[int, bytes], tuple] = {}
    skipped_names = 0
    for key, node_id, graph_u, knot_id, evidence, representation_id in mappings:
        name = str(knot_id).removeprefix("knot:")
        knot_pk = knot_pks.get(name)
        if knot_pk is None:
            skipped_names += 1
            continue
        row = (knot_pk, bytes(key), int(node_id), graph_u, evidence, representation_id)
        link_key = (knot_pk, bytes(key))
        previous = links.get(link_key)
        if previous is None or (evidence_rank[evidence], representation_id) < (
            evidence_rank[previous[4]],
            previous[5],
        ):
            links[link_key] = row
    db.executemany("INSERT INTO graph_knot_links VALUES (?,?,?,?,?,?)", links.values())

    graph = sqlite3.connect(f"file:{graph_path}?mode=ro", uri=True)
    graph_nodes = {
        bytes(key): (int(node_id), u_upper)
        for key, node_id, u_upper in graph.execute(
            "SELECT k.rep_key,k.node_id,n.u_upper_bound FROM node_keys k JOIN nodes n USING(node_id)"
        )
    }
    node_names: dict[int, set[int]] = {}
    for knot_pk, key in links:
        graph_row = graph_nodes.get(key)
        if graph_row is not None:
            node_names.setdefault(graph_row[0], set()).add(knot_pk)
    unique_names = {
        node_id: next(iter(names))
        for node_id, names in node_names.items()
        if len(names) == 1
    }
    key_by_node = {node_id: key for key, (node_id, _) in graph_nodes.items()}
    inserted = 0
    for edge_id, source_node, target_node in graph.execute(
        "SELECT edge_id,source_node,target_node FROM edges WHERE cc_cost=1 ORDER BY edge_id"
    ):
        source_pk = unique_names.get(int(source_node))
        target_pk = unique_names.get(int(target_node))
        if source_pk is None or target_pk is None:
            continue
        source_key = key_by_node[int(source_node)]
        target_key = key_by_node[int(target_node)]
        claim_id = text_sha256("graph", graph_sha256, str(edge_id))
        db.execute(
            """
            INSERT INTO adjacency_claims(
                claim_id,source_id,source_knot_pk,target_knot_pk,relation,cc_cost,
                status,claim_scope,source_stopping_key,target_stopping_key,
                graph_snapshot_sha256,graph_edge_id,source_pointer
            ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)
            """,
            (
                claim_id,
                "unknotdb-proof-graph",
                source_pk,
                target_pk,
                "crossing_change",
                1,
                "replay_verified",
                "normalized_representation",
                source_key,
                target_key,
                graph_sha256,
                int(edge_id),
                f"edges.edge_id={edge_id}",
            ),
        )
        inserted += 1
    graph.close()
    return len(links), inserted, skipped_names


def import_adjacency_tsv(
    db: sqlite3.Connection,
    paths: Iterable[Path],
    knot_pks: dict[str, int],
) -> int:
    inserted = 0
    for path in paths:
        with path.open(encoding="utf-8", newline="") as stream:
            reader = csv.DictReader(stream, delimiter="\t")
            required = {
                "source_id",
                "source_url",
                "retrieved_at",
                "source_sha256",
                "source_format",
                "source_scope",
                "source_knot",
                "target_knot",
                "status",
                "source_pointer",
            }
            if not required.issubset(reader.fieldnames or ()):
                raise ValueError(f"adjacency TSV lacks required columns: {path}")
            for row in reader:
                db.execute(
                    "INSERT OR IGNORE INTO catalogue_sources VALUES (?,?,?,?,?,?,?)",
                    (
                        row["source_id"],
                        "cc_adjacency",
                        row["source_url"],
                        row["retrieved_at"],
                        row["source_sha256"],
                        row["source_format"],
                        row["source_scope"],
                    ),
                )
                registered = db.execute(
                    "SELECT source_url,retrieved_at,source_sha256,source_format,source_scope FROM catalogue_sources WHERE source_id=?",
                    (row["source_id"],),
                ).fetchone()
                expected = (
                    row["source_url"],
                    row["retrieved_at"],
                    row["source_sha256"],
                    row["source_format"],
                    row["source_scope"],
                )
                if registered != expected:
                    raise ValueError(f"conflicting source metadata: {row['source_id']}")
                source_pk = knot_pks[row["source_knot"]]
                target_pk = knot_pks[row["target_knot"]]
                status = row["status"]
                scope = row.get("claim_scope") or "knot_type"
                source_representation_pk = _external_representation(
                    db,
                    source_pk,
                    row.get("source_representation_encoding", ""),
                    row.get("source_representation", ""),
                    row["source_id"],
                    row["source_pointer"],
                )
                target_representation_pk = _external_representation(
                    db,
                    target_pk,
                    row.get("target_representation_encoding", ""),
                    row.get("target_representation", ""),
                    row["source_id"],
                    row["source_pointer"],
                )
                parts = (
                    row["source_id"],
                    row["source_knot"],
                    row["target_knot"],
                    status,
                    scope,
                    row["source_pointer"],
                )
                db.execute(
                    """
                    INSERT INTO adjacency_claims(
                        claim_id,source_id,source_knot_pk,target_knot_pk,relation,
                        cc_cost,status,claim_scope,source_representation_pk,
                        target_representation_pk,source_stopping_key,target_stopping_key,
                        crossing_locator,witness_uri,witness_sha256,
                        graph_snapshot_sha256,graph_edge_id,source_pointer
                    ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                    """,
                    (
                        text_sha256(*parts),
                        row["source_id"],
                        source_pk,
                        target_pk,
                        "crossing_change",
                        1,
                        status,
                        scope,
                        source_representation_pk,
                        target_representation_pk,
                        bytes.fromhex(row["source_stopping_key"])
                        if row.get("source_stopping_key")
                        else None,
                        bytes.fromhex(row["target_stopping_key"])
                        if row.get("target_stopping_key")
                        else None,
                        row.get("crossing_locator") or None,
                        row.get("witness_uri") or None,
                        row.get("witness_sha256") or None,
                        row.get("graph_snapshot_sha256") or None,
                        int(row["graph_edge_id"]) if row.get("graph_edge_id") else None,
                        row["source_pointer"],
                    ),
                )
                inserted += 1
    return inserted


def _external_representation(
    db: sqlite3.Connection,
    knot_pk: int,
    encoding: str,
    text: str,
    source_id: str,
    source_pointer: str,
) -> int | None:
    if not encoding and not text:
        return None
    if not encoding or not text:
        raise ValueError("external representation needs both encoding and text")
    digest = text_sha256(encoding, text)
    db.execute(
        """
        INSERT OR IGNORE INTO representations(
            knot_pk,encoding,representation_text,representation_sha256,
            source_id,source_pointer,preferred_rank
        ) VALUES (?,?,?,?,?,?,100)
        """,
        (knot_pk, encoding, text, digest, source_id, source_pointer),
    )
    row = db.execute(
        "SELECT representation_pk FROM representations WHERE knot_pk=? AND encoding=? AND representation_sha256=?",
        (knot_pk, encoding, digest),
    ).fetchone()
    return int(row[0])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--knotinfo-xls", type=Path, required=True)
    parser.add_argument("--source-id", default="knotinfo-2026-08-14")
    parser.add_argument(
        "--source-url", default="https://knotinfo.org/knotinfo_data_complete.xls"
    )
    parser.add_argument("--retrieved-at", required=True)
    parser.add_argument("--max-crossings", type=int, default=13)
    parser.add_argument("--soffice", default="soffice")
    parser.add_argument("--graph", type=Path)
    parser.add_argument("--identifications", type=Path)
    parser.add_argument("--graph-pinned-at")
    parser.add_argument("--adjacency-tsv", type=Path, action="append", default=[])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if args.max_crossings < 0:
        raise ValueError("--max-crossings must be nonnegative")
    if (args.graph is None) != (args.identifications is None):
        raise ValueError("--graph and --identifications must be supplied together")
    if args.graph is not None and not args.graph_pinned_at:
        raise ValueError("--graph-pinned-at is required with --graph")
    for output in (args.output, args.manifest, args.report):
        if output.exists():
            raise FileExistsError(f"output already exists: {output}")

    rows = load_knotinfo(args.knotinfo_xls, args.soffice, args.max_crossings)
    source_sha256 = file_sha256(args.knotinfo_xls)
    builder_sha256 = file_sha256(Path(__file__))
    graph_sha256 = file_sha256(args.graph) if args.graph else None
    identity_sha256 = (
        file_sha256(args.identifications) if args.identifications else None
    )
    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    db = sqlite3.connect(temporary)
    create_schema(db)
    meta = {
        "schema": SCHEMA,
        "proof_status": "metadata-only; only replay_verified rows reference proof edges",
        "max_crossings": str(args.max_crossings),
        "knotinfo_source_sha256": source_sha256,
        "builder_sha256": builder_sha256,
        "graph_snapshot_sha256": graph_sha256 or "none",
        "identification_sidecar_sha256": identity_sha256 or "none",
        "adjacency_semantics": "bounded-source claims; not complete Gordian neighborhoods",
    }
    db.executemany("INSERT INTO meta VALUES (?,?)", sorted(meta.items()))
    db.execute(
        "INSERT INTO catalogue_sources VALUES (?,?,?,?,?,?,?)",
        (
            args.source_id,
            "knotinfo",
            args.source_url,
            args.retrieved_at,
            source_sha256,
            "KnotInfo XLS",
            f"canonical knot table through {args.max_crossings} crossings",
        ),
    )
    if args.graph:
        db.execute(
            "INSERT INTO catalogue_sources VALUES (?,?,?,?,?,?,?)",
            (
                "unknotdb-proof-graph",
                "proof_graph",
                str(args.graph.resolve()),
                args.graph_pinned_at,
                graph_sha256,
                "Unknot DB SQLite snapshot",
                "replay-validated normalized-representation CC edges",
            ),
        )
    knot_pks = import_knotinfo(db, rows, args.source_id)
    graph_links = replay_edges = skipped_names = 0
    if args.graph:
        graph_links, replay_edges, skipped_names = import_graph_links(
            db, knot_pks, args.graph, args.identifications, graph_sha256
        )
    external_edges = import_adjacency_tsv(db, args.adjacency_tsv, knot_pks)
    db.execute("PRAGMA optimize")
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise RuntimeError(
            f"sidecar validation failed: integrity={integrity} fk={foreign_keys[:3]}"
        )
    counts = {
        table: db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
        for table in (
            "knots",
            "knot_identifiers",
            "representations",
            "property_values",
            "knot_properties",
            "graph_knot_links",
            "adjacency_claims",
        )
    }
    status_counts = dict(
        db.execute("SELECT status,count(*) FROM adjacency_claims GROUP BY status")
    )
    db.commit()
    db.close()
    os.replace(temporary, args.output)
    sidecar_sha256 = file_sha256(args.output)

    manifest = {
        "format": "unknotdb-federated-catalogue-manifest-v1",
        "builder_sha256": builder_sha256,
        "sidecar": str(args.output),
        "sidecar_sha256": sidecar_sha256,
        "sidecar_bytes": args.output.stat().st_size,
        "source": {
            "id": args.source_id,
            "url": args.source_url,
            "retrieved_at": args.retrieved_at,
            "path": str(args.knotinfo_xls.resolve()),
            "sha256": source_sha256,
        },
        "graph": None
        if args.graph is None
        else {
            "path": str(args.graph.resolve()),
            "sha256": graph_sha256,
            "bytes": args.graph.stat().st_size,
            "identifications": str(args.identifications.resolve()),
            "identifications_sha256": identity_sha256,
            "pinned_at": args.graph_pinned_at,
        },
        "max_crossings": args.max_crossings,
        "counts": counts,
        "adjacency_status_counts": status_counts,
        "external_adjacency_rows": external_edges,
        "graph_links": graph_links,
        "replay_verified_graph_edges": replay_edges,
        "graph_identification_names_outside_catalogue": skipped_names,
        "validation": {"integrity_check": integrity, "foreign_key_check_rows": 0},
        "storage": {
            "sidecar_bytes_per_knot": args.output.stat().st_size / len(rows),
            "core_plus_sidecar_bytes": (
                args.output.stat().st_size
                + (args.graph.stat().st_size if args.graph is not None else 0)
            ),
        },
    }
    manifest_temporary = args.manifest.with_name(
        f"{args.manifest.name}.tmp-{os.getpid()}"
    )
    manifest_temporary.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    os.replace(manifest_temporary, args.manifest)

    nontrivial = sum(row["name"] != "0_1" for row in rows)
    report = f"""# Federated KnotInfo catalogue and adjacency sidecar v1

- Sidecar: `{args.output}`
- SHA-256: `{sidecar_sha256}`
- Size: {args.output.stat().st_size:,} bytes ({args.output.stat().st_size / 2**20:.2f} MiB)
- Bytes per catalogue knot: {args.output.stat().st_size / len(rows):,.1f}
- Core plus sidecar: {(args.output.stat().st_size + (args.graph.stat().st_size if args.graph else 0)) / 2**20:.2f} MiB
- KnotInfo source: `{args.source_url}`
- Retrieved: `{args.retrieved_at}`; SHA-256: `{source_sha256}`
- Imported knot types: {len(rows):,} ({nontrivial:,} nontrivial), through {args.max_crossings} crossings
- Identifiers: {counts["knot_identifiers"]:,}
- Compact diagram/braid representations: {counts["representations"]:,}
- Searchable property assignments: {counts["knot_properties"]:,}
- Graph/name links: {graph_links:,}
- Replay-verified named CC edges: {replay_edges:,}
- Imported external adjacency claims: {external_edges:,}
- Integrity check: `{integrity}`; foreign-key violations: 0

The sidecar is federated metadata, not the proof graph.  KnotInfo properties and
external adjacency rows do not alter `U_upper`.  A `replay_verified` adjacency
row is only a compact named projection of an immutable edge in the pinned graph
snapshot.  Adjacency is source-bounded and must not be interpreted as a complete
Gordian neighborhood.
"""
    report_temporary = args.report.with_name(f"{args.report.name}.tmp-{os.getpid()}")
    report_temporary.write_text(report)
    os.replace(report_temporary, args.report)
    print(json.dumps(manifest["counts"], sort_keys=True))


if __name__ == "__main__":
    main()
