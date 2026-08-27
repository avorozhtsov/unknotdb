#!/usr/bin/env python3
"""Invariant and representation-map CLI for standalone Unknot DB."""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from pathlib import Path
from typing import Any


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
        raise argparse.ArgumentTypeError("word must contain comma-separated integers") from error
    if any(value == 0 for value in result):
        raise argparse.ArgumentTypeError("word cannot contain padding zero")
    return result


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
    if schema != ("unknotdb-identification-maps-v3",):
        raise ValueError("unsupported identification-map schema")
    return result


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
        print("\t".join("-" if row[column] is None else str(row[column]) for column in columns))


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
    parser.add_argument("--maps", type=Path, default=Path("outputs/unknotdb-lookup-maps-v2.sqlite"))
    parser.add_argument(
        "--fingerprints",
        type=Path,
        default=Path("outputs/unknotdb-braid-fingerprints-v3.sqlite"),
    )
    parser.add_argument(
        "--identifications",
        type=Path,
        default=Path("outputs/unknotdb-identification-maps-v3.sqlite"),
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
    find_knots.add_argument("--invariant", action="append", type=constraint, required=True)
    find_knots.add_argument("--limit", type=int, default=100)

    find_representations = commands.add_parser("find-representations-by-invariant")
    find_representations.add_argument(
        "--invariant", action="append", type=constraint, required=True
    )
    find_representations.add_argument("--feature", action="append", type=constraint, default=[])
    find_representations.add_argument(
        "--fingerprint", action="append", type=constraint, default=[]
    )
    find_representations.add_argument("--limit", type=int, default=100)

    find_features = commands.add_parser("find-representations-by-feature")
    find_features.add_argument("--feature", action="append", type=constraint, required=True)
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

    gaps = commands.add_parser("list-identification-gaps")
    gaps.add_argument(
        "--kind", choices=("candidate", "unidentified", "all"), default="all"
    )
    gaps.add_argument("--limit", type=int, default=100)

    args = parser.parse_args()
    if hasattr(args, "limit") and args.limit <= 0:
        raise ValueError("--limit must be positive")

    if args.command == "compute-braid-invariants":
        if args.strands < 1 or any(abs(value) >= args.strands for value in args.word):
            raise ValueError("braid word has a generator outside the declared strand count")
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
            {"kind": "knot_invariant", "name": "determinant", "value": determinant(args.word, args.strands)},
            {"kind": "knot_invariant", "name": "alexander", "value": json.dumps(alexander, separators=(",", ":"))},
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
                    "value": json.dumps(to_pairs(jones_polynomial(args.word, args.strands)), separators=(",", ":")),
                }
            )
        rows.extend(
            (
                {"kind": "representation_feature", "name": "braid_strands", "value": args.strands},
                {"kind": "representation_feature", "name": "word_length", "value": len(args.word)},
                {"kind": "representation_feature", "name": "writhe", "value": sum(args.word)},
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
            rows = [
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
                    WHERE representation_id IN ({','.join('?' for _ in identities)})
                    ORDER BY representation_id
                    """,
                    identities,
                )
            ] if identities else []
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

    if args.command in {"identification-for-representation", "list-identification-gaps"}:
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
        knot_id = args.name if args.name.startswith("knot:") else f"knot:{args.name}"
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
                if fingerprint_matches is not None and identity not in fingerprint_matches:
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
            {"name": name, "kind": "knot_invariant", "type": value_type, "scope": scope, "definition": definition}
            for name, value_type, scope, definition in db.execute(
                "SELECT invariant_id,value_type,scope,definition FROM invariant_definitions ORDER BY invariant_id"
            )
        ]
        if args.include_features:
            rows.extend(
                {"name": name, "kind": "representation_feature", "type": "integer", "scope": "representation", "definition": definition}
                for name, definition in db.execute(
                    "SELECT feature_id,definition FROM representation_feature_definitions ORDER BY feature_id"
                )
            )
        emit(rows, args.json)
    db.close()


if __name__ == "__main__":
    main()
