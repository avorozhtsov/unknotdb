#!/usr/bin/env python3
"""Invariant and representation-map CLI for standalone Unknot DB."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sqlite3
import sys
from pathlib import Path
from typing import Any

SNAPPY_KNOT_NAME = re.compile(r"^K?(\d+)([an])(\d+)$", re.IGNORECASE)


def canonical_knot_id(name: str) -> str:
    name = name.removeprefix("knot:")
    match = SNAPPY_KNOT_NAME.fullmatch(name.replace("_", ""))
    if match:
        crossings, kind, index = match.groups()
        return f"knot:{int(crossings)}{kind.lower()}_{int(index)}"
    return f"knot:{name}"


def constraint(text: str) -> tuple[str, str]:
    if "=" not in text:
        raise argparse.ArgumentTypeError("constraint must be NAME=VALUE")
    name, value = text.split("=", 1)
    if not name or not value:
        raise argparse.ArgumentTypeError("constraint name and value must be non-empty")
    return name, value


def word(text: str) -> tuple[int, ...]:
    if not text:
        return ()
    try:
        result = tuple(int(value) for value in text.split(","))
    except ValueError as error:
        raise argparse.ArgumentTypeError(
            "word must contain comma-separated integers"
        ) from error
    if any(value == 0 for value in result):
        raise argparse.ArgumentTypeError("word cannot contain padding zero")
    return result


def canonical_pd_text(text: str) -> str:
    """Canonicalize the catalogue JSON-array spelling of a PD diagram."""
    try:
        value = json.loads(text)
    except json.JSONDecodeError as error:
        raise ValueError("PD must be a JSON array of four-integer crossings") from error
    if not isinstance(value, list) or any(
        not isinstance(crossing, list)
        or len(crossing) != 4
        or any(
            not isinstance(label, int) or isinstance(label, bool) for label in crossing
        )
        for crossing in value
    ):
        raise ValueError("PD must be a JSON array of four-integer crossings")
    return json.dumps(value, separators=(",", ":"))


def representation_digest(encoding: str, representation: str) -> bytes:
    return hashlib.sha256(
        encoding.encode() + b"\0" + representation.encode() + b"\0"
    ).digest()


def pd_rows_for_graph(
    identifications: sqlite3.Connection,
    federation: sqlite3.Connection,
    key_or_node: str,
    limit: int,
) -> list[dict[str, Any]]:
    if key_or_node.isdigit() and len(key_or_node) != 64:
        predicate = "v.node_id=?"
        parameter: Any = int(key_or_node)
    else:
        if len(key_or_node) != 64:
            raise ValueError("graph key must be 64 hexadecimal characters")
        try:
            parameter = bytes.fromhex(key_or_node)
        except ValueError as error:
            raise ValueError("graph key is not hexadecimal") from error
        predicate = "v.rep_key=?"
    mappings = list(
        identifications.execute(
            f"""
            SELECT lower(hex(v.rep_key)),v.node_id,m.knot_id,m.mirror_bit,
                   m.evidence_class,m.mapping_status
            FROM graph_vertices v JOIN graph_vertex_knot_map m USING(rep_key)
            WHERE {predicate}
            """,
            (parameter,),
        )
    )
    rows: list[dict[str, Any]] = []
    for rep_key, node_id, knot_id, mirror_bit, evidence_class, status in mappings:
        canonical_id = str(knot_id).removeprefix("knot:")
        for pd, digest, source_id, source_pointer in federation.execute(
            """
            SELECT r.representation_text,lower(hex(r.representation_sha256)),
                   r.source_id,r.source_pointer
            FROM representations r JOIN knots k USING(knot_pk)
            WHERE k.canonical_id=? AND r.encoding='pd'
            ORDER BY r.preferred_rank,r.representation_pk LIMIT ?
            """,
            (canonical_id, limit - len(rows)),
        ):
            rows.append(
                {
                    "relation": "same_knot_type",
                    "exact_conversion": False,
                    "graph_node_id": node_id,
                    "rep_key": rep_key,
                    "knot_id": knot_id,
                    "mirror_bit": mirror_bit,
                    "graph_evidence_class": evidence_class,
                    "graph_mapping_status": status,
                    "pd": pd,
                    "pd_sha256": digest,
                    "pd_source": source_id,
                    "pd_source_pointer": source_pointer,
                }
            )
            if len(rows) == limit:
                return rows
    return rows


def graph_rows_for_pd(
    identifications: sqlite3.Connection,
    federation: sqlite3.Connection,
    representation: str | None,
    digest_hex: str | None,
    limit: int,
) -> list[dict[str, Any]]:
    if bool(representation) == bool(digest_hex):
        raise ValueError("provide exactly one PD text or --sha256")
    pd = canonical_pd_text(representation) if representation is not None else None
    try:
        digest = (
            bytes.fromhex(digest_hex)
            if digest_hex is not None
            else representation_digest("pd", pd or "")
        )
    except ValueError as error:
        raise ValueError("PD SHA-256 is not hexadecimal") from error
    if len(digest) != 32:
        raise ValueError("PD SHA-256 must contain 64 hexadecimal characters")
    catalogue_rows = list(
        federation.execute(
            """
            SELECT k.canonical_id,r.representation_text,
                   lower(hex(r.representation_sha256)),r.source_id,r.source_pointer
            FROM representations r JOIN knots k USING(knot_pk)
            WHERE r.encoding='pd' AND r.representation_sha256=?
            ORDER BY k.canonical_id
            """,
            (digest,),
        )
    )
    rows: list[dict[str, Any]] = []
    for (
        canonical_id,
        stored_pd,
        stored_digest,
        source_id,
        source_pointer,
    ) in catalogue_rows:
        knot_id = canonical_knot_id(canonical_id)
        for (
            rep_key,
            node_id,
            mirror_bit,
            evidence_class,
            status,
        ) in identifications.execute(
            """
            SELECT lower(hex(rep_key)),graph_node_id,mirror_bit,
                   evidence_class,mapping_status
            FROM knot_representation_postings
            WHERE knot_id=? AND representation_namespace='graph'
            ORDER BY graph_node_id,rep_key LIMIT ?
            """,
            (knot_id, limit - len(rows)),
        ):
            rows.append(
                {
                    "relation": "same_knot_type",
                    "exact_conversion": False,
                    "knot_id": knot_id,
                    "pd": stored_pd,
                    "pd_sha256": stored_digest,
                    "pd_source": source_id,
                    "pd_source_pointer": source_pointer,
                    "graph_node_id": node_id,
                    "rep_key": rep_key,
                    "mirror_bit": mirror_bit,
                    "graph_evidence_class": evidence_class,
                    "graph_mapping_status": status,
                }
            )
            if len(rows) == limit:
                return rows
    return rows


def connection(path: Path) -> sqlite3.Connection:
    result = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    schema = result.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema not in {
        ("unknotdb-lookup-maps-v1",),
        ("unknotdb-lookup-maps-v2",),
    }:
        raise ValueError("unsupported lookup-map schema")
    return result


def fingerprint_connection(path: Path) -> sqlite3.Connection:
    result = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    schema = result.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema not in {
        ("unknotdb-braid-fingerprints-v1",),
        ("unknotdb-braid-fingerprints-v2",),
    }:
        raise ValueError("unsupported braid-fingerprint schema")
    return result


def identification_connection(path: Path) -> sqlite3.Connection:
    result = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    schema = result.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema not in {
        ("unknotdb-identification-maps-v3",),
        ("unknotdb-graph-identification-v4",),
        ("unknotdb-graph-identification-v5",),
    }:
        raise ValueError("unsupported identification-map schema")
    return result


def federation_connection(path: Path) -> sqlite3.Connection:
    result = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    schema = result.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema != ("unknotdb-federated-catalogue-v1",):
        raise ValueError("unsupported federated catalogue schema")
    return result


def lower_bound_connection(path: Path) -> sqlite3.Connection:
    result = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    schema = result.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema != ("unknotdb-lower-bound-sidecar-v0",):
        raise ValueError("unsupported lower-bound sidecar schema")
    return result


def resolve_knot(db: sqlite3.Connection, identifier: str, scheme: str | None) -> int:
    if scheme:
        rows = list(
            db.execute(
                "SELECT knot_pk FROM knot_identifiers WHERE scheme=? AND identifier=?",
                (scheme, identifier),
            )
        )
    else:
        rows = list(
            db.execute(
                "SELECT DISTINCT knot_pk FROM knot_identifiers WHERE identifier=?",
                (identifier,),
            )
        )
    if not rows:
        raise ValueError(f"unknown catalogue identifier: {identifier}")
    if len(rows) != 1:
        raise ValueError(
            f"ambiguous catalogue identifier: {identifier}; specify --scheme"
        )
    return int(rows[0][0])


def federated_knot_rows(db: sqlite3.Connection, knot_pk: int) -> list[dict[str, Any]]:
    name, crossing, table_kind = db.execute(
        "SELECT canonical_id,crossing_number,table_kind FROM knots WHERE knot_pk=?",
        (knot_pk,),
    ).fetchone()
    rows: list[dict[str, Any]] = [
        {
            "kind": "catalogue_knot",
            "name": name,
            "value": crossing,
            "detail": table_kind,
        }
    ]
    rows.extend(
        {
            "kind": "identifier",
            "name": scheme,
            "value": identifier,
            "detail": role,
        }
        for scheme, identifier, role in db.execute(
            "SELECT scheme,identifier,role FROM knot_identifiers WHERE knot_pk=? ORDER BY role,scheme,identifier",
            (knot_pk,),
        )
    )
    rows.extend(
        {
            "kind": "representation",
            "name": encoding,
            "value": text,
            "detail": f"sha256={digest} rank={rank}",
        }
        for encoding, text, digest, rank in db.execute(
            "SELECT encoding,representation_text,lower(hex(representation_sha256)),preferred_rank FROM representations WHERE knot_pk=? ORDER BY preferred_rank,encoding",
            (knot_pk,),
        )
    )
    rows.extend(
        {
            "kind": "catalogue_property",
            "name": name,
            "value": value,
            "detail": source_id,
        }
        for name, value, source_id in db.execute(
            """
            SELECT d.property_name,v.value_text,p.source_id
            FROM knot_properties p JOIN property_definitions d USING(property_id)
            JOIN property_values v USING(value_id)
            WHERE p.knot_pk=? ORDER BY d.property_name,p.source_id
            """,
            (knot_pk,),
        )
    )
    rows.extend(
        {
            "kind": "proof_graph_link",
            "name": f"node:{node_id}",
            "value": key,
            "detail": f"U_upper={u_upper} evidence={evidence}",
        }
        for key, node_id, u_upper, evidence in db.execute(
            "SELECT lower(hex(stopping_key)),graph_node_id,graph_u_upper,evidence_class FROM graph_knot_links WHERE knot_pk=? ORDER BY evidence_class,stopping_key",
            (knot_pk,),
        )
    )
    return rows


def emit(rows: list[dict[str, Any]], as_json: bool) -> None:
    if as_json:
        print(json.dumps(rows, indent=2, sort_keys=True))
        return
    if not rows:
        print("no matches")
        return
    columns = list(rows[0])
    print("\t".join(columns))
    for row in rows:
        print(
            "\t".join(
                "-" if row[column] is None else str(row[column]) for column in columns
            )
        )


def invariant_rows(db: sqlite3.Connection, knot_id: str) -> list[dict[str, Any]]:
    return [
        {
            "invariant": invariant_id,
            "value": value,
            "type": value_type,
            "scope": scope,
            "provenance": provenance,
        }
        for invariant_id, value, value_type, scope, provenance in db.execute(
            """
            SELECT v.invariant_id,v.value_text,v.value_type,d.scope,v.provenance
            FROM knot_invariant_values v
            JOIN invariant_definitions d USING(invariant_id)
            WHERE v.knot_id=?
            ORDER BY v.invariant_id
            """,
            (knot_id,),
        )
    ]


def find_matching_knots(
    db: sqlite3.Connection, wanted: list[tuple[str, str]], limit: int
) -> list[str]:
    if not wanted:
        raise ValueError("at least one --invariant is required")
    if len(set(wanted)) != len(wanted):
        raise ValueError("duplicate invariant constraint")
    values_sql = ",".join("(?,?)" for _ in wanted)
    parameters: list[Any] = [item for pair in wanted for item in pair]
    parameters.extend((len(wanted), limit))
    return [
        row[0]
        for row in db.execute(
            f"""
            WITH wanted(invariant_id,value_text) AS (VALUES {values_sql})
            SELECT v.knot_id
            FROM knot_invariant_values v
            JOIN wanted w USING(invariant_id,value_text)
            GROUP BY v.knot_id
            HAVING count(*)=?
            ORDER BY v.knot_id
            LIMIT ?
            """,
            parameters,
        )
    ]


def find_matching_representations(
    db: sqlite3.Connection, wanted: list[tuple[str, str]], limit: int
) -> list[str]:
    if not wanted:
        raise ValueError("at least one --fingerprint is required")
    if len(set(wanted)) != len(wanted):
        raise ValueError("duplicate fingerprint constraint")
    values_sql = ",".join("(?,?)" for _ in wanted)
    parameters: list[Any] = [item for pair in wanted for item in pair]
    parameters.extend((len(wanted), limit))
    return [
        row[0]
        for row in db.execute(
            f"""
            WITH wanted(fingerprint_id,value_text) AS (VALUES {values_sql})
            SELECT p.representation_id
            FROM representation_fingerprints p
            JOIN fingerprint_values v USING(value_key)
            JOIN fingerprint_definitions d USING(fingerprint_key)
            JOIN wanted w USING(fingerprint_id,value_text)
            GROUP BY p.representation_id
            HAVING count(*)=?
            ORDER BY p.representation_id
            LIMIT ?
            """,
            parameters,
        )
    ]


def main() -> None:
    parser = argparse.ArgumentParser(prog="unknotdb")
    parser.add_argument(
        "--maps", type=Path, default=Path("release-v0.10.1/lookup.sqlite")
    )
    parser.add_argument(
        "--fingerprints",
        type=Path,
        default=Path("release-v0.10.1/fingerprints.sqlite"),
    )
    parser.add_argument(
        "--identifications",
        type=Path,
        default=Path("release-v0.10.1/identification.sqlite"),
    )
    parser.add_argument(
        "--federation",
        type=Path,
        default=Path("release-v0.10.1/federation.sqlite"),
    )
    parser.add_argument(
        "--lower-bounds",
        type=Path,
        default=Path("outputs/unknotdb-dgkt-lower-bounds-v0.sqlite"),
    )
    parser.add_argument("--json", action="store_true")
    commands = parser.add_subparsers(dest="command", required=True)

    knot = commands.add_parser("invariants-for-knot")
    knot.add_argument("name", help="for example 3_1 or knot:3_1")

    representation = commands.add_parser("invariants-for-representation")
    representation.add_argument("representation_id")

    braid = commands.add_parser("compute-braid-invariants")
    braid.add_argument("strands", type=int)
    braid.add_argument("word", type=word)
    braid.add_argument("--rf-src", type=Path, required=True)
    braid.add_argument("--skip-jones", action="store_true")

    find_knots = commands.add_parser("find-knots-by-invariant")
    find_knots.add_argument(
        "--invariant", action="append", type=constraint, required=True
    )
    find_knots.add_argument("--limit", type=int, default=100)

    find_representations = commands.add_parser("find-representations-by-invariant")
    find_representations.add_argument(
        "--invariant", action="append", type=constraint, required=True
    )
    find_representations.add_argument(
        "--feature", action="append", type=constraint, default=[]
    )
    find_representations.add_argument(
        "--fingerprint", action="append", type=constraint, default=[]
    )
    find_representations.add_argument("--limit", type=int, default=100)

    find_features = commands.add_parser("find-representations-by-feature")
    find_features.add_argument(
        "--feature", action="append", type=constraint, required=True
    )
    find_features.add_argument("--limit", type=int, default=100)

    definitions = commands.add_parser("list-invariants")
    definitions.add_argument("--include-features", action="store_true")

    representation_fingerprints = commands.add_parser("fingerprints-for-representation")
    representation_fingerprints.add_argument("representation_id")

    find_fingerprints = commands.add_parser("find-representations-by-fingerprint")
    find_fingerprints.add_argument(
        "--fingerprint", action="append", type=constraint, required=True
    )
    find_fingerprints.add_argument("--limit", type=int, default=100)

    commands.add_parser("list-fingerprints")

    identify = commands.add_parser("identification-for-representation")
    identify.add_argument("representation_id")

    identify_graph = commands.add_parser("identification-for-graph")
    identify_graph.add_argument("key_or_node")

    pd_for_graph = commands.add_parser("pd-for-graph")
    pd_for_graph.add_argument("key_or_node")
    pd_for_graph.add_argument("--limit", type=int, default=100)

    graph_for_pd = commands.add_parser("graph-for-pd")
    graph_for_pd.add_argument("pd", nargs="?")
    graph_for_pd.add_argument("--sha256")
    graph_for_pd.add_argument("--limit", type=int, default=100)

    postings = commands.add_parser("representations-for-knot")
    postings.add_argument("name", help="for example 3_1 or knot:3_1")
    postings.add_argument(
        "--namespace", choices=("source", "graph", "all"), default="all"
    )
    postings.add_argument("--limit", type=int, default=100)

    equivalence_candidates = commands.add_parser("list-equivalence-candidates")
    equivalence_candidates.add_argument(
        "--status",
        choices=("pending", "attested", "verified", "all"),
        default="pending",
    )
    equivalence_candidates.add_argument("--limit", type=int, default=100)

    gaps = commands.add_parser("list-identification-gaps")
    gaps.add_argument(
        "--kind", choices=("candidate", "unidentified", "all"), default="all"
    )
    gaps.add_argument("--limit", type=int, default=100)

    knot_show = commands.add_parser("knot-show")
    knot_show.add_argument("identifier")
    knot_show.add_argument("--scheme")

    resolve_identifier = commands.add_parser("resolve-identifier")
    resolve_identifier.add_argument("identifier")
    resolve_identifier.add_argument("--scheme")

    resolve_representation = commands.add_parser("resolve-representation")
    resolve_representation.add_argument("encoding")
    resolve_representation.add_argument("representation", nargs="?")
    resolve_representation.add_argument("--sha256")
    resolve_representation.add_argument("--limit", type=int, default=100)

    neighbors = commands.add_parser("neighbors")
    neighbors.add_argument("identifier")
    neighbors.add_argument("--scheme")
    neighbors.add_argument(
        "--status",
        choices=("claimed", "diagram_attested", "replay_verified", "all"),
        default="all",
    )
    neighbors.add_argument("--direction", choices=("out", "in", "both"), default="both")
    neighbors.add_argument(
        "--claims",
        action="store_true",
        help="show every representation-level claim instead of knot-type groups",
    )
    neighbors.add_argument("--limit", type=int, default=100)

    commands.add_parser("catalogue-sources")
    commands.add_parser("catalogue-coverage")

    find_catalogue = commands.add_parser("find-knots-by-catalogue-property")
    find_catalogue.add_argument("--property", type=constraint, required=True)
    find_catalogue.add_argument("--limit", type=int, default=100)

    lower_for_knot = commands.add_parser("lower-bound-for-knot")
    lower_for_knot.add_argument("name", help="for example 11n3 or knot:11n_3")

    find_lower = commands.add_parser("find-knots-by-lower-bound")
    find_lower.add_argument("--min", type=int, default=0)
    find_lower.add_argument("--exact-only", action="store_true")
    find_lower.add_argument("--method")
    find_lower.add_argument("--limit", type=int, default=100)

    commands.add_parser("lower-bound-sources")

    args = parser.parse_args()
    if hasattr(args, "limit") and args.limit <= 0:
        raise ValueError("--limit must be positive")

    if args.command == "compute-braid-invariants":
        if args.strands < 1 or any(abs(value) >= args.strands for value in args.word):
            raise ValueError(
                "braid word has a generator outside the declared strand count"
            )
        sys.path.insert(0, str(args.rf_src.resolve()))
        from rf_knots.invariants import (  # type: ignore[import-not-found]
            alexander_polynomial,
            determinant,
            jones_polynomial,
            signature,
            to_pairs,
        )

        alexander = to_pairs(alexander_polynomial(args.word, args.strands))
        signature_value = signature(args.word, args.strands)
        rows = [
            {
                "kind": "knot_invariant",
                "name": "determinant",
                "value": determinant(args.word, args.strands),
            },
            {
                "kind": "knot_invariant",
                "name": "alexander",
                "value": json.dumps(alexander, separators=(",", ":")),
            },
            {"kind": "knot_invariant", "name": "signature", "value": signature_value},
            {
                "kind": "rigorous_lower_bound",
                "name": "murasugi_u_lower",
                "value": None if signature_value is None else abs(signature_value) // 2,
            },
        ]
        if not args.skip_jones:
            rows.append(
                {
                    "kind": "knot_invariant",
                    "name": "jones",
                    "value": json.dumps(
                        to_pairs(jones_polynomial(args.word, args.strands)),
                        separators=(",", ":"),
                    ),
                }
            )
        rows.extend(
            (
                {
                    "kind": "representation_feature",
                    "name": "braid_strands",
                    "value": args.strands,
                },
                {
                    "kind": "representation_feature",
                    "name": "word_length",
                    "value": len(args.word),
                },
                {
                    "kind": "representation_feature",
                    "name": "writhe",
                    "value": sum(args.word),
                },
                {
                    "kind": "representation_feature",
                    "name": "representation_l10",
                    "value": 10 * args.strands + len(args.word),
                },
            )
        )
        emit(rows, args.json)
        return

    if args.command in {
        "lower-bound-for-knot",
        "find-knots-by-lower-bound",
        "lower-bound-sources",
    }:
        lower_bounds = lower_bound_connection(args.lower_bounds)
        if args.command == "lower-bound-for-knot":
            knot_id = canonical_knot_id(args.name)
            rows = [
                {
                    "knot_id": knot,
                    "u_lower": lower,
                    "retained_upper": upper,
                    "exact": bool(exact),
                    "trust_status": trust,
                    "method": method,
                    "source": source,
                    "source_pointer": pointer,
                    "repository_url": repository,
                    "source_commit": commit,
                    "work_title": work,
                    "certificate_pointer": f"{artifact}:{line}",
                    "certificate_status": certificate_status,
                }
                for knot, lower, upper, exact, trust, method, source, pointer, repository, commit, work, artifact, line, certificate_status in lower_bounds.execute(
                    """
                    SELECT c.knot_id,c.u_lower,c.retained_upper,c.interval_exact,
                           c.trust_status,m.display_name,c.source_id,c.source_pointer,
                           s.repository_url,s.source_commit,s.work_title,
                           e.artifact_path,e.csv_line,e.verification_status
                    FROM lower_bound_claims c JOIN sources s USING(source_id)
                    JOIN claim_evidence e USING(claim_id)
                    JOIN methods m USING(method_id)
                    WHERE c.knot_id=?
                    ORDER BY m.method_id,e.artifact_path,e.csv_line
                    """,
                    (knot_id,),
                )
            ]
        elif args.command == "find-knots-by-lower-bound":
            clauses = ["c.u_lower>=?"]
            parameters: list[Any] = [args.min]
            if args.exact_only:
                clauses.append("c.interval_exact=1")
            if args.method:
                clauses.append("m.method_id=?")
                parameters.append(args.method)
            parameters.append(args.limit)
            rows = [
                {
                    "knot_id": knot,
                    "u_lower": lower,
                    "retained_upper": upper,
                    "exact": bool(exact),
                    "method": method,
                    "trust_status": trust,
                }
                for knot, lower, upper, exact, method, trust in lower_bounds.execute(
                    f"""
                    SELECT DISTINCT c.knot_id,c.u_lower,c.retained_upper,
                           c.interval_exact,m.method_id,c.trust_status
                    FROM lower_bound_claims c JOIN claim_evidence e USING(claim_id)
                    JOIN methods m USING(method_id)
                    WHERE {" AND ".join(clauses)}
                    ORDER BY c.u_lower DESC,c.knot_id,m.method_id LIMIT ?
                    """,
                    parameters,
                )
            ]
        else:
            rows = [
                {
                    "source_id": source,
                    "repository_url": repository,
                    "source_commit": commit,
                    "commit_date": commit_date,
                    "work_title": work,
                    "work_url": work_url,
                    "citation_pointer": citation,
                    "erratum_pointer": erratum,
                    "license": license_name,
                }
                for source, repository, commit, commit_date, work, work_url, citation, erratum, license_name in lower_bounds.execute(
                    """
                    SELECT source_id,repository_url,source_commit,commit_date,
                           work_title,work_url,citation_pointer,erratum_pointer,license
                    FROM sources ORDER BY source_id
                    """
                )
            ]
        lower_bounds.close()
        emit(rows, args.json)
        return

    if args.command in {
        "pd-for-graph",
        "graph-for-pd",
        "knot-show",
        "resolve-identifier",
        "resolve-representation",
        "neighbors",
        "catalogue-sources",
        "catalogue-coverage",
        "find-knots-by-catalogue-property",
    }:
        federation = federation_connection(args.federation)
        if args.command in {"pd-for-graph", "graph-for-pd"}:
            identifications = identification_connection(args.identifications)
        if args.command in {"knot-show", "resolve-identifier", "neighbors"}:
            knot_pk = resolve_knot(federation, args.identifier, args.scheme)
        if args.command == "pd-for-graph":
            rows = pd_rows_for_graph(
                identifications, federation, args.key_or_node, args.limit
            )
            identifications.close()
        elif args.command == "graph-for-pd":
            rows = graph_rows_for_pd(
                identifications, federation, args.pd, args.sha256, args.limit
            )
            identifications.close()
        elif args.command == "knot-show":
            rows = federated_knot_rows(federation, knot_pk)
        elif args.command == "resolve-identifier":
            rows = [
                {
                    "knot_id": name,
                    "crossing_number": crossing,
                    "table_kind": table_kind,
                }
                for name, crossing, table_kind in federation.execute(
                    "SELECT canonical_id,crossing_number,table_kind FROM knots WHERE knot_pk=?",
                    (knot_pk,),
                )
            ]
        elif args.command == "resolve-representation":
            if bool(args.representation) == bool(args.sha256):
                raise ValueError("provide exactly one representation text or --sha256")
            digest = (
                bytes.fromhex(args.sha256)
                if args.sha256
                else representation_digest(args.encoding, args.representation)
            )
            rows = [
                {
                    "knot_id": knot_id,
                    "encoding": encoding,
                    "representation": representation,
                    "sha256": sha256,
                    "source_id": source_id,
                }
                for knot_id, encoding, representation, sha256, source_id in federation.execute(
                    """
                    SELECT k.canonical_id,r.encoding,r.representation_text,
                           lower(hex(r.representation_sha256)),r.source_id
                    FROM representations r JOIN knots k USING(knot_pk)
                    WHERE r.encoding=? AND r.representation_sha256=?
                    ORDER BY k.canonical_id LIMIT ?
                    """,
                    (args.encoding, digest, args.limit),
                )
            ]
        elif args.command == "neighbors":
            status_clause = "" if args.status == "all" else " AND a.status=?"
            status_params: list[Any] = [] if args.status == "all" else [args.status]
            directions = (
                ("out", "in") if args.direction == "both" else (args.direction,)
            )
            rows = []
            for direction in directions:
                if direction == "out" and args.claims:
                    query = f"""
                        SELECT t.canonical_id,a.status,a.claim_scope,a.source_id,
                               a.crossing_locator,a.graph_edge_id,
                               lower(hex(a.claim_id)),1
                        FROM adjacency_claims a JOIN knots t
                          ON t.knot_pk=a.target_knot_pk
                        WHERE a.source_knot_pk=?{status_clause}
                        ORDER BY a.status,t.canonical_id,a.claim_id LIMIT ?
                    """
                elif direction == "in" and args.claims:
                    query = f"""
                        SELECT s.canonical_id,a.status,a.claim_scope,a.source_id,
                               a.crossing_locator,a.graph_edge_id,
                               lower(hex(a.claim_id)),1
                        FROM adjacency_claims a JOIN knots s
                          ON s.knot_pk=a.source_knot_pk
                        WHERE a.target_knot_pk=?{status_clause}
                        ORDER BY a.status,s.canonical_id,a.claim_id LIMIT ?
                    """
                elif direction == "out":
                    query = f"""
                        SELECT t.canonical_id,a.status,a.claim_scope,a.source_id,
                               NULL,min(a.graph_edge_id),NULL,count(*)
                        FROM adjacency_claims a JOIN knots t
                          ON t.knot_pk=a.target_knot_pk
                        WHERE a.source_knot_pk=?{status_clause}
                        GROUP BY t.canonical_id,a.status,a.claim_scope,a.source_id
                        ORDER BY a.status,t.canonical_id LIMIT ?
                    """
                else:
                    query = f"""
                        SELECT s.canonical_id,a.status,a.claim_scope,a.source_id,
                               NULL,min(a.graph_edge_id),NULL,count(*)
                        FROM adjacency_claims a JOIN knots s
                          ON s.knot_pk=a.source_knot_pk
                        WHERE a.target_knot_pk=?{status_clause}
                        GROUP BY s.canonical_id,a.status,a.claim_scope,a.source_id
                        ORDER BY a.status,s.canonical_id LIMIT ?
                    """
                params = [knot_pk, *status_params, args.limit - len(rows)]
                rows.extend(
                    {
                        "direction": direction,
                        "neighbor": neighbor,
                        "status": status,
                        "scope": scope,
                        "source": source,
                        "crossing": crossing,
                        "graph_edge_id": edge_id,
                        "claim_id": claim_id,
                        "claim_count": claim_count,
                    }
                    for neighbor, status, scope, source, crossing, edge_id, claim_id, claim_count in federation.execute(
                        query, params
                    )
                )
                if len(rows) >= args.limit:
                    break
        elif args.command == "catalogue-sources":
            rows = [
                {
                    "source_id": source_id,
                    "kind": kind,
                    "retrieved_at": retrieved,
                    "sha256": digest,
                    "scope": scope,
                    "url": url,
                }
                for source_id, kind, retrieved, digest, scope, url in federation.execute(
                    "SELECT source_id,catalogue_kind,retrieved_at,source_sha256,source_scope,source_url FROM catalogue_sources ORDER BY source_id"
                )
            ]
        elif args.command == "catalogue-coverage":
            total, linked, represented, properties = federation.execute(
                """
                SELECT count(*),
                       count(*) FILTER (WHERE EXISTS (SELECT 1 FROM graph_knot_links g WHERE g.knot_pk=k.knot_pk)),
                       count(*) FILTER (WHERE EXISTS (SELECT 1 FROM representations r WHERE r.knot_pk=k.knot_pk)),
                       count(*) FILTER (WHERE EXISTS (SELECT 1 FROM knot_properties p WHERE p.knot_pk=k.knot_pk))
                FROM knots k
                """
            ).fetchone()
            rows = [
                {
                    "catalogue_knots": total,
                    "with_graph_link": linked,
                    "with_representation": represented,
                    "with_searchable_properties": properties,
                    "graph_coverage_percent": f"{100 * linked / total:.3f}",
                }
            ]
        else:
            property_name, wanted_value = args.property
            rows = [
                {"knot_id": knot_id, "property": property_name, "value": value}
                for knot_id, value in federation.execute(
                    """
                    SELECT k.canonical_id,v.value_text
                    FROM property_definitions d JOIN property_values v USING(property_id)
                    JOIN knot_properties p USING(property_id,value_id)
                    JOIN knots k USING(knot_pk)
                    WHERE d.property_name=? AND v.value_text=?
                    ORDER BY k.crossing_number,k.canonical_id LIMIT ?
                    """,
                    (property_name, wanted_value, args.limit),
                )
            ]
        federation.close()
        emit(rows, args.json)
        return

    if args.command in {
        "fingerprints-for-representation",
        "find-representations-by-fingerprint",
        "list-fingerprints",
    }:
        fingerprints = fingerprint_connection(args.fingerprints)
        if args.command == "fingerprints-for-representation":
            rows = [
                {
                    "fingerprint": fingerprint_id,
                    "value": value,
                    "type": value_type,
                    "scope": scope,
                    "valid_under": valid_under,
                    "safe_use": safe_use,
                }
                for fingerprint_id, value, value_type, scope, valid_under, safe_use in fingerprints.execute(
                    """
                    SELECT d.fingerprint_id,v.value_text,v.value_type,d.scope,
                           d.valid_under,d.safe_use
                    FROM representation_fingerprints p
                    JOIN fingerprint_values v USING(value_key)
                    JOIN fingerprint_definitions d USING(fingerprint_key)
                    WHERE p.representation_id=? ORDER BY d.fingerprint_id
                    """,
                    (args.representation_id,),
                )
            ]
        elif args.command == "find-representations-by-fingerprint":
            identities = find_matching_representations(
                fingerprints, args.fingerprint, args.limit
            )
            rows = (
                [
                    {
                        "representation_id": identity,
                        "knot_id": knot_id,
                        "identification_class": identification_class,
                        "stopping_key": key,
                        "graph_node_id": node_id,
                    }
                    for identity, knot_id, identification_class, key, node_id in fingerprints.execute(
                        f"""
                    SELECT representation_id,knot_id,identification_class,
                           lower(hex(stopping_key)),graph_node_id
                    FROM representation_map
                    WHERE representation_id IN ({",".join("?" for _ in identities)})
                    ORDER BY representation_id
                    """,
                        identities,
                    )
                ]
                if identities
                else []
            )
        else:
            rows = [
                {
                    "name": name,
                    "type": value_type,
                    "scope": scope,
                    "valid_under": valid_under,
                    "safe_use": safe_use,
                    "definition": definition,
                }
                for name, value_type, scope, valid_under, safe_use, definition in fingerprints.execute(
                    """
                    SELECT fingerprint_id,value_type,scope,valid_under,safe_use,definition
                    FROM fingerprint_definitions ORDER BY fingerprint_id
                    """
                )
            ]
        fingerprints.close()
        emit(rows, args.json)
        return

    if args.command in {
        "identification-for-representation",
        "identification-for-graph",
        "representations-for-knot",
        "list-equivalence-candidates",
        "list-identification-gaps",
    }:
        identifications = identification_connection(args.identifications)
        if args.command == "identification-for-representation":
            rows = [
                {
                    "kind": "effective_mapping",
                    "knot_id": knot_id,
                    "evidence_class": evidence_class,
                    "status": status,
                    "mirror_bit": mirror_bit,
                }
                for knot_id, mirror_bit, evidence_class, status in identifications.execute(
                    """
                    SELECT knot_id,mirror_bit,evidence_class,mapping_status
                    FROM effective_representation_knot_map WHERE representation_id=?
                    """,
                    (args.representation_id,),
                )
            ]
            if not rows:
                rows.extend(
                    {
                        "kind": "candidate_only",
                        "knot_id": knot_id,
                        "evidence_class": "candidate",
                        "status": f"invariant-rank-{rank}",
                        "mirror_bit": mirror_bit,
                    }
                    for knot_id, mirror_bit, rank in identifications.execute(
                        """
                        SELECT candidate_knot_id,mirror_bit,candidate_rank
                        FROM representation_knot_candidates
                        WHERE representation_id=? ORDER BY candidate_rank,candidate_knot_id
                        """,
                        (args.representation_id,),
                    )
                )
        elif args.command == "identification-for-graph":
            if args.key_or_node.isdigit() and len(args.key_or_node) != 64:
                predicate = "v.node_id=?"
                parameter: Any = int(args.key_or_node)
            else:
                if len(args.key_or_node) != 64:
                    raise ValueError("graph key must be 64 hexadecimal characters")
                try:
                    parameter = bytes.fromhex(args.key_or_node)
                except ValueError as error:
                    raise ValueError("graph key is not hexadecimal") from error
                predicate = "v.rep_key=?"
            rows = [
                {
                    "kind": "effective_mapping",
                    "rep_key": key,
                    "graph_node_id": node_id,
                    "knot_id": knot_id,
                    "evidence_class": evidence_class,
                    "status": status,
                    "mirror_bit": mirror_bit,
                    "cc0_component": component,
                }
                for key, node_id, component, knot_id, mirror_bit, evidence_class, status in identifications.execute(
                    f"""
                    SELECT lower(hex(v.rep_key)),v.node_id,lower(hex(v.cc0_component_key)),
                           m.knot_id,m.mirror_bit,m.evidence_class,m.mapping_status
                    FROM graph_vertices v LEFT JOIN graph_vertex_knot_map m USING(rep_key)
                    WHERE {predicate}
                    """,
                    (parameter,),
                )
            ]
        elif args.command == "representations-for-knot":
            knot_id = canonical_knot_id(args.name)
            namespace_clause = (
                "" if args.namespace == "all" else "AND representation_namespace=?"
            )
            parameters: tuple[Any, ...] = (
                (knot_id, args.limit)
                if args.namespace == "all"
                else (knot_id, args.namespace, args.limit)
            )
            rows = [
                {
                    "knot_id": found_knot,
                    "namespace": namespace,
                    "representation": reference,
                    "rep_key": key,
                    "graph_node_id": node_id,
                    "mirror_bit": mirror_bit,
                    "evidence_class": evidence_class,
                    "status": status,
                }
                for found_knot, namespace, reference, key, node_id, mirror_bit, evidence_class, status in identifications.execute(
                    f"""
                    SELECT knot_id,representation_namespace,representation_ref,
                           lower(hex(rep_key)),graph_node_id,mirror_bit,evidence_class,mapping_status
                    FROM knot_representation_postings
                    WHERE knot_id=? {namespace_clause}
                    ORDER BY representation_namespace,representation_ref LIMIT ?
                    """,
                    parameters,
                )
            ]
        elif args.command == "list-equivalence-candidates":
            status_clause = "" if args.status == "all" else "WHERE c.status=?"
            parameters = (
                (args.limit,) if args.status == "all" else (args.status, args.limit)
            )
            rows = [
                {
                    "rank": rank,
                    "status": status,
                    "candidate_knot_id": knot_id,
                    "left_rep_key": left,
                    "right_rep_key": right,
                    "common_target_key": target,
                    "external_method": method,
                }
                for rank, status, knot_id, left, right, target, method in identifications.execute(
                    f"""
                    SELECT c.candidate_rank,c.status,c.candidate_knot_id,
                           lower(hex(c.left_rep_key)),lower(hex(c.right_rep_key)),
                           lower(hex(c.common_target_key)),
                           (SELECT group_concat(DISTINCT x.method)
                            FROM graph_equivalence_checks x
                            WHERE x.candidate_id=c.candidate_id)
                    FROM graph_equivalence_candidates c {status_clause}
                    ORDER BY c.candidate_rank,c.candidate_id LIMIT ?
                    """,
                    parameters,
                )
            ]
        else:
            clauses = []
            if args.kind == "candidate":
                clauses.append("c.representation_id IS NOT NULL")
            elif args.kind == "unidentified":
                clauses.append("c.representation_id IS NULL")
            where = f"WHERE {' AND '.join(clauses)}" if clauses else ""
            rows = [
                {
                    "representation_id": identity,
                    "gap_kind": "candidate" if candidate_count else "unidentified",
                    "candidate_count": candidate_count,
                    "graph_node_id": node_id,
                    "graph_u_upper": u_upper,
                }
                for identity, node_id, u_upper, candidate_count in identifications.execute(
                    f"""
                    SELECT u.representation_id,u.graph_node_id,u.graph_u_upper,
                           count(c.candidate_knot_id)
                    FROM unidentified_representations u
                    LEFT JOIN representation_knot_candidates c USING(representation_id)
                    {where}
                    GROUP BY u.representation_id
                    ORDER BY u.representation_id LIMIT ?
                    """,
                    (args.limit,),
                )
            ]
        identifications.close()
        emit(rows, args.json)
        return

    db = connection(args.maps)
    if args.command == "invariants-for-knot":
        knot_id = canonical_knot_id(args.name)
        emit(invariant_rows(db, knot_id), args.json)
    elif args.command == "invariants-for-representation":
        identifications = identification_connection(args.identifications)
        effective = identifications.execute(
            """
            SELECT knot_id,evidence_class,mapping_status
            FROM effective_representation_knot_map WHERE representation_id=?
            """,
            (args.representation_id,),
        ).fetchone()
        identifications.close()
        header = db.execute(
            """
            SELECT lower(hex(g.stopping_key)),g.graph_node_id,g.graph_u_upper,g.status
            FROM representation_graph_map g
            WHERE g.representation_id=?
            """,
            (args.representation_id,),
        ).fetchone()
        rows = []
        if header:
            rows.append(
                {
                    "kind": "mapping",
                    "name": effective[0] if effective else None,
                    "value": header[0],
                    "detail": (
                        f"graph_node={header[1]} U_upper={header[2]} status={header[3]} "
                        f"identification={effective[1]}:{effective[2]}"
                        if effective
                        else f"graph_node={header[1]} U_upper={header[2]} status={header[3]} identification=none"
                    ),
                }
            )
        if effective:
            rows.extend(
                {
                    "kind": "knot_invariant",
                    "name": invariant_id,
                    "value": value,
                    "detail": "knot_dictionary",
                }
                for invariant_id, value in db.execute(
                    """
                    SELECT invariant_id,value_text FROM knot_invariant_values
                    WHERE knot_id=? ORDER BY invariant_id
                    """,
                    (effective[0],),
                )
            )
        rows.extend(
            {
                "kind": "representation_feature",
                "name": feature_id,
                "value": value,
                "detail": "representation-dependent",
            }
            for feature_id, value in db.execute(
                """
                SELECT feature_id,value_integer FROM representation_feature_values
                WHERE representation_id=? ORDER BY feature_id
                """,
                (args.representation_id,),
            )
        )
        emit(rows, args.json)
    elif args.command == "find-knots-by-invariant":
        knot_ids = find_matching_knots(db, args.invariant, args.limit)
        emit([{"knot_id": knot_id} for knot_id in knot_ids], args.json)
    elif args.command == "find-representations-by-invariant":
        knot_ids = find_matching_knots(db, args.invariant, 1_000_000)
        fingerprint_matches = None
        if args.fingerprint:
            fingerprints = fingerprint_connection(args.fingerprints)
            fingerprint_matches = set(
                find_matching_representations(fingerprints, args.fingerprint, 1_000_000)
            )
            fingerprints.close()
        rows = []
        identifications = identification_connection(args.identifications)
        for knot_id in knot_ids:
            for identity in (
                row[0]
                for row in identifications.execute(
                    """
                    SELECT representation_id FROM effective_representation_knot_map
                    WHERE knot_id=? ORDER BY representation_id
                    """,
                    (knot_id,),
                )
            ):
                graph_row = db.execute(
                    """
                    SELECT lower(hex(stopping_key)),graph_node_id
                    FROM representation_graph_map WHERE representation_id=?
                    """,
                    (identity,),
                ).fetchone()
                key, node_id = graph_row if graph_row else (None, None)
                if (
                    fingerprint_matches is not None
                    and identity not in fingerprint_matches
                ):
                    continue
                features = dict(
                    db.execute(
                        "SELECT feature_id,CAST(value_integer AS TEXT) FROM representation_feature_values WHERE representation_id=?",
                        (identity,),
                    )
                )
                if all(features.get(name) == value for name, value in args.feature):
                    rows.append(
                        {
                            "knot_id": knot_id,
                            "representation_id": identity,
                            "stopping_key": key,
                            "graph_node_id": node_id,
                        }
                    )
                    if len(rows) >= args.limit:
                        break
            if len(rows) >= args.limit:
                break
        identifications.close()
        emit(rows, args.json)
    elif args.command == "find-representations-by-feature":
        wanted = args.feature
        values_sql = ",".join("(?,CAST(? AS INTEGER))" for _ in wanted)
        parameters: list[Any] = [item for pair in wanted for item in pair]
        parameters.extend((len(wanted), args.limit))
        rows = [
            {
                "representation_id": identity,
                "knot_id": knot_id,
                "stopping_key": key,
                "graph_node_id": node_id,
            }
            for identity, knot_id, key, node_id in db.execute(
                f"""
                WITH wanted(feature_id,value_integer) AS (VALUES {values_sql}),
                matched AS (
                    SELECT f.representation_id FROM representation_feature_values f
                    JOIN wanted w USING(feature_id,value_integer)
                    GROUP BY f.representation_id HAVING count(*)=?
                )
                SELECT x.representation_id,k.knot_id,lower(hex(g.stopping_key)),g.graph_node_id
                FROM matched x
                LEFT JOIN representation_knot_map k USING(representation_id)
                LEFT JOIN representation_graph_map g USING(representation_id)
                ORDER BY x.representation_id LIMIT ?
                """,
                parameters,
            )
        ]
        emit(rows, args.json)
    elif args.command == "list-invariants":
        rows = [
            {
                "name": name,
                "kind": "knot_invariant",
                "type": value_type,
                "scope": scope,
                "definition": definition,
            }
            for name, value_type, scope, definition in db.execute(
                "SELECT invariant_id,value_type,scope,definition FROM invariant_definitions ORDER BY invariant_id"
            )
        ]
        if args.include_features:
            rows.extend(
                {
                    "name": name,
                    "kind": "representation_feature",
                    "type": "integer",
                    "scope": "representation",
                    "definition": definition,
                }
                for name, definition in db.execute(
                    "SELECT feature_id,definition FROM representation_feature_definitions ORDER BY feature_id"
                )
            )
        emit(rows, args.json)
    db.close()


if __name__ == "__main__":
    main()
