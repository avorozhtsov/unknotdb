#!/usr/bin/env python3
"""Build a compact, provenance-bearing DGKT lower-bound sidecar."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import sqlite3
import subprocess
import sys
from collections import Counter, defaultdict
from collections.abc import Iterable
from datetime import datetime, timezone
from pathlib import Path

INTERVAL = re.compile(r"^\[(\d+),(\d+)\]$")
SOURCE_METHODS = {
    "Alexander module": (
        "alexander_module",
        "lower_bounds/alexander_module/new_certificates.csv",
    ),
    "CW": ("casson_walker", "lower_bounds/cw/cw_scan.csv"),
    "Montesinos correction term": (
        "montesinos_correction_terms",
        None,
    ),
    "D&D": (
        "owens_determinant_discriminant",
        "lower_bounds/owens/valid_determinant_group_certificates.csv",
    ),
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_knot_id(name: str) -> str:
    cleaned = name.strip()
    rolfsen = re.fullmatch(r"(\d+)_(\d+)", cleaned)
    if rolfsen:
        return f"knot:{int(rolfsen.group(1))}_{int(rolfsen.group(2))}"
    compact = cleaned.replace("_", "")
    match = re.fullmatch(r"(\d+)([an])(\d+)", compact, re.IGNORECASE)
    if match:
        crossings, kind, index = match.groups()
        return f"knot:{int(crossings)}{kind.lower()}_{int(index)}"
    raise ValueError(f"unsupported knot identifier: {name!r}")


def parse_interval(text: str) -> tuple[int, int]:
    match = INTERVAL.fullmatch(text.replace(" ", ""))
    if not match:
        raise ValueError(f"invalid interval: {text!r}")
    lower, upper = map(int, match.groups())
    if lower > upper:
        raise ValueError(f"reversed interval: {text!r}")
    return lower, upper


def csv_rows(root: Path, relative: str) -> list[tuple[int, dict[str, str]]]:
    with (root / relative).open(newline="", encoding="utf-8") as handle:
        return list(enumerate(csv.DictReader(handle), start=2))


def evidence_index(root: Path) -> dict[str, dict[str, list[tuple[str, int, str]]]]:
    """Return method -> knot -> (path, line, canonical row JSON)."""
    result: dict[str, dict[str, list[tuple[str, int, str]]]] = defaultdict(
        lambda: defaultdict(list)
    )

    def add(
        method: str,
        relative: str,
        rows: Iterable[tuple[int, dict[str, str]]],
        knot_field: str,
    ) -> None:
        for line, row in rows:
            result[method][canonical_knot_id(row[knot_field])].append(
                (relative, line, json.dumps(row, sort_keys=True, separators=(",", ":")))
            )

    relative = SOURCE_METHODS["Alexander module"][1]
    assert relative is not None
    add("Alexander module", relative, csv_rows(root, relative), "knot")

    relative = SOURCE_METHODS["CW"][1]
    assert relative is not None
    add(
        "CW",
        relative,
        (item for item in csv_rows(root, relative) if item[1]["obstructs_u_1"] == "1"),
        "knot_id",
    )

    for relative, flag in (
        (
            (
                "lower_bounds/montesinos_u1/"
                "montesinos_d_obstruction_scan_u1_targets_up_to_13.csv"
            ),
            "obstructed_full",
        ),
        (
            (
                "lower_bounds/montesinos_signature_sharp/"
                "montesinos_spin_certificates.csv"
            ),
            "lower_endpoint_obstructed",
        ),
    ):
        add(
            "Montesinos correction term",
            relative,
            (item for item in csv_rows(root, relative) if item[1][flag] == "True"),
            "knot",
        )

    relative = SOURCE_METHODS["D&D"][1]
    assert relative is not None
    add("D&D", relative, csv_rows(root, relative), "knot")
    return result


def git(root: Path, *arguments: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(root), *arguments], text=True
    ).strip()


def build(args: argparse.Namespace) -> dict[str, object]:
    root = args.source_root.resolve()
    final_path = root / "all_updated_intervals.csv"
    citation_path = root / "CITATION.cff"
    erratum_path = root / "ERRATUM.md"
    if not all(path.is_file() for path in (final_path, citation_path, erratum_path)):
        raise ValueError("source root is not a complete DGKT repository checkout")
    if git(root, "status", "--porcelain"):
        raise ValueError("DGKT source checkout must be clean for reproducible import")

    source_commit = git(root, "rev-parse", "HEAD")
    source_tree = git(root, "rev-parse", "HEAD^{tree}")
    commit_date = git(root, "show", "-s", "--format=%cI", "HEAD")
    verifier_result = json.loads(
        subprocess.check_output(
            [sys.executable, str(root / "verify_repository.py")], text=True
        )
    )
    if verifier_result.get("status") != "OK":
        raise ValueError("DGKT repository verifier did not return status=OK")
    evidence = evidence_index(root)
    final_rows = csv_rows(root, "all_updated_intervals.csv")

    claims = []
    support = []
    seen: set[str] = set()
    for line, row in final_rows:
        knot_id = canonical_knot_id(row["knot"])
        if knot_id in seen:
            raise ValueError(f"duplicate final claim for {knot_id}")
        seen.add(knot_id)
        input_lower, input_upper = parse_interval(row["input_interval"])
        lower, upper = parse_interval(row["updated_interval"])
        if lower <= input_lower or upper != input_upper:
            raise ValueError(f"not a strict lower-bound improvement at line {line}")
        methods = row["source"].split("; ")
        if any(method not in SOURCE_METHODS for method in methods):
            raise ValueError(f"unknown method at line {line}: {methods}")
        claim_id = hashlib.sha256(
            f"dgkt\0{source_commit}\0{knot_id}\0{lower}\0{upper}".encode()
        ).digest()
        claims.append(
            (
                claim_id,
                knot_id,
                int(row["crossing_number"]),
                input_lower,
                input_upper,
                lower,
                upper,
                int(lower == upper),
                row["source"],
                "externally_attested",
                line,
            )
        )
        for method in methods:
            matches = evidence[method].get(knot_id, [])
            if not matches:
                raise ValueError(f"no {method} certificate row for {knot_id}")
            method_id = SOURCE_METHODS[method][0]
            for relative, certificate_line, row_json in matches:
                row_sha = hashlib.sha256(row_json.encode()).hexdigest()
                support.append(
                    (
                        claim_id,
                        method_id,
                        relative,
                        certificate_line,
                        row_sha,
                        "source-consistent-not-independently-recomputed",
                    )
                )

    if len(claims) != 1943:
        raise ValueError(f"expected 1943 final claims, found {len(claims)}")
    exact = sum(row[7] for row in claims)
    if exact != 1677:
        raise ValueError(f"expected 1677 exact intervals, found {exact}")

    artifact_paths = {
        "all_updated_intervals.csv",
        "CITATION.cff",
        "ERRATUM.md",
        "STATUS.md",
        "verify_repository.py",
        *(item[2] for item in support),
    }
    artifacts = [
        (relative, sha256(root / relative), (root / relative).stat().st_size)
        for relative in sorted(artifact_paths)
    ]

    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f"{output.name}.tmp-{os.getpid()}")
    if temporary.exists():
        temporary.unlink()
    db = sqlite3.connect(temporary)
    db.execute("PRAGMA foreign_keys=ON")
    db.executescript(
        """
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=FULL;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE sources(
            source_id TEXT PRIMARY KEY,
            repository_url TEXT NOT NULL,
            source_commit TEXT NOT NULL CHECK(length(source_commit)=40),
            source_tree TEXT NOT NULL CHECK(length(source_tree)=40),
            commit_date TEXT NOT NULL,
            retrieved_at TEXT NOT NULL,
            work_title TEXT NOT NULL,
            work_url TEXT,
            citation_pointer TEXT NOT NULL,
            erratum_pointer TEXT NOT NULL,
            license TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE source_artifacts(
            source_id TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            sha256 TEXT NOT NULL CHECK(length(sha256)=64),
            byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
            PRIMARY KEY(source_id,relative_path),
            FOREIGN KEY(source_id) REFERENCES sources(source_id)
        ) WITHOUT ROWID;
        CREATE TABLE methods(
            method_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            claim_scope TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE lower_bound_claims(
            claim_id BLOB PRIMARY KEY CHECK(length(claim_id)=32),
            knot_id TEXT NOT NULL UNIQUE,
            crossing_number INTEGER NOT NULL CHECK(crossing_number >= 0),
            prior_lower INTEGER NOT NULL CHECK(prior_lower >= 0),
            prior_upper INTEGER NOT NULL CHECK(prior_upper >= prior_lower),
            u_lower INTEGER NOT NULL CHECK(u_lower > prior_lower),
            retained_upper INTEGER NOT NULL CHECK(retained_upper >= u_lower),
            interval_exact INTEGER NOT NULL CHECK(interval_exact IN (0,1)),
            method_summary TEXT NOT NULL,
            trust_status TEXT NOT NULL CHECK(trust_status IN ('externally_attested','verified')),
            source_id TEXT NOT NULL,
            source_pointer TEXT NOT NULL,
            FOREIGN KEY(source_id) REFERENCES sources(source_id)
        );
        CREATE INDEX lower_bound_reverse ON lower_bound_claims(u_lower,knot_id);
        CREATE INDEX exact_value_reverse ON lower_bound_claims(retained_upper,knot_id)
            WHERE interval_exact=1;
        CREATE TABLE claim_evidence(
            claim_id BLOB NOT NULL,
            method_id TEXT NOT NULL,
            artifact_path TEXT NOT NULL,
            csv_line INTEGER NOT NULL CHECK(csv_line >= 2),
            canonical_row_sha256 TEXT NOT NULL CHECK(length(canonical_row_sha256)=64),
            verification_status TEXT NOT NULL,
            PRIMARY KEY(claim_id,method_id,artifact_path,csv_line),
            FOREIGN KEY(claim_id) REFERENCES lower_bound_claims(claim_id),
            FOREIGN KEY(method_id) REFERENCES methods(method_id)
        ) WITHOUT ROWID;
        CREATE VIEW effective_lower_bounds AS
            SELECT knot_id,u_lower,retained_upper,interval_exact,trust_status,
                   source_id,source_pointer,method_summary
            FROM lower_bound_claims;
        CREATE VIEW catalogue_u_claims AS
            SELECT knot_id,u_lower,retained_upper AS u_upper,
                   CASE interval_exact WHEN 1 THEN 'exact' ELSE 'range' END AS claim_kind
            FROM lower_bound_claims;
        """
    )
    retrieved_at = datetime.now(timezone.utc).replace(microsecond=0).isoformat()
    meta = {
        "schema": "unknotdb-lower-bound-sidecar-v0",
        "claim_count": str(len(claims)),
        "exact_interval_count": str(exact),
        "nonexact_interval_count": str(len(claims) - exact),
        "trust_boundary": "cited lower bounds; no proof-graph mutation",
        "source_verifier": "passed",
        "source_verifier_result": json.dumps(
            verifier_result, sort_keys=True, separators=(",", ":")
        ),
    }
    db.executemany("INSERT INTO meta VALUES (?,?)", sorted(meta.items()))
    db.execute(
        "INSERT INTO sources VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        (
            "dgkt-unknot-v2.1.0",
            "https://github.com/dtubbenhauer/unknot",
            source_commit,
            source_tree,
            commit_date,
            retrieved_at,
            "Machine learning methods and unknotting numbers",
            None,
            "CITATION.cff",
            "ERRATUM.md",
            "Unlicense",
        ),
    )
    db.executemany(
        "INSERT INTO source_artifacts VALUES (?,?,?,?)",
        [("dgkt-unknot-v2.1.0", *row) for row in artifacts],
    )
    db.executemany(
        "INSERT INTO methods VALUES (?,?,?)",
        (
            ("alexander_module", "finite-field Alexander module", "knot lower bound"),
            ("casson_walker", "Casson-Walker obstruction", "knot lower bound"),
            (
                "montesinos_correction_terms",
                "Montesinos correction terms",
                "knot lower bound",
            ),
            (
                "owens_determinant_discriminant",
                "Owens determinant/discriminant",
                "knot lower bound",
            ),
        ),
    )
    db.executemany(
        "INSERT INTO lower_bound_claims VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        [
            (*row[:10], "dgkt-unknot-v2.1.0", f"all_updated_intervals.csv:{row[10]}")
            for row in claims
        ],
    )
    db.executemany("INSERT INTO claim_evidence VALUES (?,?,?,?,?,?)", support)
    db.commit()
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise ValueError(f"SQLite validation failed: {integrity}, {foreign_keys}")
    db.execute("VACUUM")
    db.close()
    os.replace(temporary, output)

    method_counts = Counter()
    for row in claims:
        method_counts.update(row[8].split("; "))
    report: dict[str, object] = {
        "schema": "unknotdb-dgkt-lower-bound-import-report-v0",
        "output": str(output),
        "output_sha256": sha256(output),
        "output_bytes": output.stat().st_size,
        "source_repository": "https://github.com/dtubbenhauer/unknot",
        "source_commit": source_commit,
        "source_tree": source_tree,
        "work_title": "Machine learning methods and unknotting numbers",
        "claims": len(claims),
        "exact_intervals": exact,
        "improved_lower_bounds": len(claims) - exact,
        "support_rows": len(support),
        "method_claim_counts": dict(sorted(method_counts.items())),
        "verification": {
            "source_repository_verifier": "passed",
            "all_final_claims_have_certificate_rows": True,
            "sqlite_integrity": integrity,
            "foreign_key_errors": len(foreign_keys),
            "mathematical_trust_status": "externally_attested",
        },
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return report


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(build(args), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
