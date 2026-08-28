#!/usr/bin/env python3
"""Import one-crossing Brittenham records into a federated sidecar.

The published archives enumerate every subset of crossing changes in standard
alternating 12- and 13-crossing projections.  This importer streams the ZIPs,
keeps only rows at Hamming distance one (modulo global mirror), and stores them
as ``diagram_attested`` claims.  It never promotes them to proof-graph edges.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import os
import re
import shutil
import sqlite3
import time
import zipfile
from collections import Counter
from pathlib import Path

SCHEMA = "unknotdb-federated-catalogue-v1"
MEMBER_RE = re.compile(r"(?P<n>12|13)_(?:[3-9]|10|11|12|13|unk)\.txt$")


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


def flatten_dt(text: str) -> tuple[int, ...]:
    value = ast.literal_eval(text)
    while (
        isinstance(value, (list, tuple))
        and len(value) == 1
        and isinstance(value[0], (list, tuple))
    ):
        value = value[0]
    if not isinstance(value, (list, tuple)):
        raise TypeError(f"not a DT sequence: {text}")
    return tuple(int(item) for item in value)


def dt_text(values: tuple[int, ...] | list[int]) -> str:
    return "[" + ",".join(str(value) for value in values) + "]"


def split_prime_line(line: str) -> tuple[tuple[int, ...] | None, list[int]]:
    if line.startswith("unknot"):
        return None, [int(value) for value in line.split()[1:]]
    if not line.startswith("{"):
        raise ValueError("unexpected prime-result line")
    close = line.index("}")
    target = tuple(int(value) for value in line[1:close].split())
    return target, [int(value) for value in line[close + 1 :].split()]


def distance_and_position(
    dt: list[int], crossings: int
) -> tuple[int, int | None, bool]:
    if len(dt) != crossings:
        raise ValueError("DT length differs from declared crossing count")
    negative = [index for index, value in enumerate(dt) if value < 0]
    if len(negative) <= crossings - len(negative):
        return len(negative), negative[0] if len(negative) == 1 else None, False
    positive = [index for index, value in enumerate(dt) if value > 0]
    return len(positive), positive[0] if len(positive) == 1 else None, True


def knot_and_dt_maps(
    db: sqlite3.Connection,
) -> tuple[dict[str, int], dict[tuple[int, ...], str]]:
    knots = {
        name: int(pk)
        for pk, name in db.execute("SELECT knot_pk,canonical_id FROM knots")
    }
    dt_map: dict[tuple[int, ...], str] = {}
    for name, text in db.execute(
        """
        SELECT k.canonical_id,r.representation_text
        FROM representations r JOIN knots k USING(knot_pk)
        WHERE r.encoding='dt'
        """
    ):
        try:
            code = flatten_dt(text)
        except (SyntaxError, ValueError, TypeError):
            continue
        dt_map[code] = name
        dt_map[tuple(-value for value in code)] = name
    return knots, dt_map


def alternating_13_map(archive: Path) -> dict[tuple[int, ...], str]:
    result: dict[tuple[int, ...], str] = {}
    with zipfile.ZipFile(archive) as source:
        member = next(
            info for info in source.infolist() if info.filename.endswith("/13_13.txt")
        )
        with source.open(member) as stream:
            for raw in stream:
                target, rest = split_prime_line(raw.decode("ascii").strip())
                crossings, source_index, *changed_dt = rest
                distance, _, _ = distance_and_position(changed_dt, crossings)
                if distance != 0 or target is None:
                    continue
                name = f"13a_{source_index}"
                code = target[1:]
                previous = result.setdefault(code, name)
                if previous != name:
                    raise ValueError("conflicting 13-crossing canonical DT mapping")
                result[tuple(-value for value in code)] = name
    return result


def identified_13_map(archive: Path) -> dict[tuple[int, ...], str]:
    """Map every 13-crossing projection in the companion SnapPy result file."""
    result: dict[tuple[int, ...], str] = {}
    name_re = re.compile(r"\[K(13[an])(\d+)\(0,0\)\]")
    with zipfile.ZipFile(archive) as source:
        member = next(
            info
            for info in source.infolist()
            if info.filename.endswith("/13cr_identrerun-sorted.txt")
        )
        with source.open(member) as stream:
            for raw in stream:
                line = raw.decode("ascii").strip()
                match = name_re.search(line)
                bracketed = re.findall(r"\[[^\]]*\]", line)
                if match is None or not bracketed:
                    continue
                code_text = bracketed[-1]
                if not re.search(r"\d", code_text):
                    continue
                code = flatten_dt(code_text)
                name = f"{match.group(1)}_{int(match.group(2))}"
                previous = result.setdefault(code, name)
                if previous != name:
                    raise ValueError("conflicting 13-crossing SnapPy identification")
                result[tuple(-value for value in code)] = name
    return result


def ensure_representation(
    db: sqlite3.Connection,
    knot_pk: int,
    text: str,
    source_id: str,
    source_pointer: str,
    rank: int,
) -> int:
    encoding = "brittenham_knotscape_dt_v1"
    digest = text_sha256(encoding, text)
    db.execute(
        """
        INSERT OR IGNORE INTO representations(
            knot_pk,encoding,representation_text,representation_sha256,
            source_id,source_pointer,preferred_rank
        ) VALUES (?,?,?,?,?,?,?)
        """,
        (knot_pk, encoding, text, digest, source_id, source_pointer, rank),
    )
    row = db.execute(
        """
        SELECT representation_pk FROM representations
        WHERE knot_pk=? AND encoding=? AND representation_sha256=?
        """,
        (knot_pk, encoding, digest),
    ).fetchone()
    return int(row[0])


def count_composite_one_change(archive: Path, crossings: int) -> int:
    count = 0
    suffix = f"/{crossings}_connsum.txt"
    with zipfile.ZipFile(archive) as source:
        member = next(
            info for info in source.infolist() if info.filename.endswith(suffix)
        )
        with source.open(member) as stream:
            for raw in stream:
                line = raw.decode("ascii").strip()
                tail = line[line.rindex("}") + 1 :]
                rest = [int(value) for value in tail.split()]
                declared, _, *changed_dt = rest
                distance, _, _ = distance_and_position(changed_dt, declared)
                count += distance == 1
    return count


def import_archive(
    db: sqlite3.Connection,
    archive: Path,
    expected_sha256: str,
    source_id: str,
    source_url: str,
    retrieved_at: str,
    knot_pks: dict[str, int],
    dt_map: dict[tuple[int, ...], str],
    map_13: dict[tuple[int, ...], str],
    identified_13: dict[tuple[int, ...], str],
) -> Counter[str]:
    actual_sha256 = file_sha256(archive)
    if actual_sha256 != expected_sha256:
        raise ValueError(f"archive SHA-256 mismatch: {archive}")
    db.execute(
        "INSERT INTO catalogue_sources VALUES (?,?,?,?,?,?,?)",
        (
            source_id,
            "cc_adjacency",
            source_url,
            retrieved_at,
            actual_sha256,
            "Brittenham sliced ZIP",
            "all crossing subsets; importer retains only distance-one prime/unknot rows",
        ),
    )
    stats: Counter[str] = Counter()
    with zipfile.ZipFile(archive) as source:
        members = sorted(
            (info for info in source.infolist() if MEMBER_RE.search(info.filename)),
            key=lambda info: info.filename,
        )
        for member in members:
            with source.open(member) as stream:
                for line_number, raw in enumerate(stream, 1):
                    stats["rows_scanned"] += 1
                    line = raw.decode("ascii").strip()
                    target_code, rest = split_prime_line(line)
                    crossings, source_index, *changed_dt = rest
                    distance, position, mirrored = distance_and_position(
                        changed_dt, crossings
                    )
                    if distance != 1:
                        continue
                    stats["distance_one_rows"] += 1
                    source_name = f"{crossings}a_{source_index}"
                    source_pk = knot_pks.get(source_name)
                    if source_pk is None:
                        stats["missing_source_name"] += 1
                        continue
                    if target_code is None:
                        target_name = "0_1"
                        target_dt = None
                    else:
                        target_dt = target_code[1:]
                        target_name = (
                            identified_13.get(tuple(changed_dt))
                            or map_13.get(target_dt)
                            or dt_map.get(target_dt)
                            or dt_map.get(tuple(-value for value in target_dt))
                        )
                    if target_name is None:
                        stats["unmapped_target"] += 1
                        continue
                    target_pk = knot_pks[target_name]
                    pointer = f"{member.filename}:line={line_number}"
                    base_dt = tuple(abs(value) for value in changed_dt)
                    source_representation_pk = ensure_representation(
                        db, source_pk, dt_text(base_dt), source_id, pointer, 10
                    )
                    target_representation_pk = None
                    if target_dt is not None:
                        target_representation_pk = ensure_representation(
                            db,
                            target_pk,
                            dt_text(target_dt),
                            source_id,
                            pointer,
                            11,
                        )
                    successor = (
                        [-value for value in changed_dt] if mirrored else changed_dt
                    )
                    assert position is not None
                    crossing_locator = json.dumps(
                        {
                            "codec": "dt-entry-sign-change-v1",
                            "entry_index": position,
                            "odd_label": 2 * position + 1,
                            "even_label": abs(successor[position]),
                            "global_mirror_applied": mirrored,
                            "raw_successor_dt": successor,
                        },
                        separators=(",", ":"),
                        sort_keys=True,
                    )
                    claim_id = text_sha256(
                        "brittenham-distance-one-v1",
                        actual_sha256,
                        member.filename,
                        str(line_number),
                        source_name,
                        target_name,
                        crossing_locator,
                    )
                    db.execute(
                        """
                        INSERT INTO adjacency_claims(
                            claim_id,source_id,source_knot_pk,target_knot_pk,
                            relation,cc_cost,status,claim_scope,
                            source_representation_pk,target_representation_pk,
                            crossing_locator,source_pointer
                        ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
                        """,
                        (
                            claim_id,
                            source_id,
                            source_pk,
                            target_pk,
                            "crossing_change",
                            1,
                            "diagram_attested",
                            "fixed_diagram",
                            source_representation_pk,
                            target_representation_pk,
                            crossing_locator,
                            pointer,
                        ),
                    )
                    stats["inserted"] += 1
    stats["composite_distance_one_skipped"] = count_composite_one_change(
        archive, int(source_id.split("-")[1])
    )
    return stats


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--archive-12", type=Path, required=True)
    parser.add_argument("--archive-13", type=Path, required=True)
    parser.add_argument("--sha256-12", required=True)
    parser.add_argument("--sha256-13", required=True)
    parser.add_argument("--identifications-13-zip", type=Path, required=True)
    parser.add_argument("--identifications-13-sha256", required=True)
    parser.add_argument("--retrieved-at", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    for output in (args.output, args.manifest, args.report):
        if output.exists():
            raise FileExistsError(f"output already exists: {output}")

    started = time.monotonic()
    parent_sha256 = file_sha256(args.input)
    builder_sha256 = file_sha256(Path(__file__))
    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    shutil.copyfile(args.input, temporary)
    db = sqlite3.connect(temporary)
    db.execute("PRAGMA foreign_keys=ON")
    schema = db.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema != (SCHEMA,):
        raise ValueError("unsupported parent sidecar schema")
    knot_pks, dt_map = knot_and_dt_maps(db)
    map_13 = alternating_13_map(args.archive_13)
    identification_sha256 = file_sha256(args.identifications_13_zip)
    if identification_sha256 != args.identifications_13_sha256:
        raise ValueError("13-crossing identification archive SHA-256 mismatch")
    identified_13 = identified_13_map(args.identifications_13_zip)
    db.execute(
        "INSERT INTO catalogue_sources VALUES (?,?,?,?,?,?,?)",
        (
            "brittenham-13-snappy-identification-2017",
            "representation_identification",
            "https://www.math.unl.edu/~mbrittenham2/unknottingsearch/database/13_crossing_knots.zip",
            args.retrieved_at,
            identification_sha256,
            "Brittenham full ZIP; 13cr_identrerun-sorted.txt",
            "SnapPy identifications for 13-crossing projections",
        ),
    )
    sources = (
        (
            args.archive_12,
            args.sha256_12,
            "brittenham-12-sliced-2017",
            "https://www.math.unl.edu/~mbrittenham2/unknottingsearch/database/12_crossing_knots_sliced.zip",
        ),
        (
            args.archive_13,
            args.sha256_13,
            "brittenham-13-sliced-2017",
            "https://www.math.unl.edu/~mbrittenham2/unknottingsearch/database/13_crossing_knots_sliced.zip",
        ),
    )
    per_source = {}
    for archive, expected_sha, source_id, source_url in sources:
        per_source[source_id] = dict(
            import_archive(
                db,
                archive,
                expected_sha,
                source_id,
                source_url,
                args.retrieved_at,
                knot_pks,
                dt_map,
                map_13,
                identified_13,
            )
        )
    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        (
            ("parent_sidecar_sha256", parent_sha256),
            ("brittenham_importer_sha256", builder_sha256),
            ("brittenham_adjacency_semantics", "distance-one-modulo-global-mirror-v1"),
        ),
    )
    db.execute("PRAGMA optimize")
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise RuntimeError(
            f"validation failed: integrity={integrity} fk={foreign_keys[:3]}"
        )
    totals = {
        "knots": db.execute("SELECT count(*) FROM knots").fetchone()[0],
        "representations": db.execute(
            "SELECT count(*) FROM representations"
        ).fetchone()[0],
        "adjacency_claims": db.execute(
            "SELECT count(*) FROM adjacency_claims"
        ).fetchone()[0],
        "diagram_attested": db.execute(
            "SELECT count(*) FROM adjacency_claims WHERE status='diagram_attested'"
        ).fetchone()[0],
        "replay_verified": db.execute(
            "SELECT count(*) FROM adjacency_claims WHERE status='replay_verified'"
        ).fetchone()[0],
        "distinct_named_pairs": db.execute(
            "SELECT count(*) FROM (SELECT DISTINCT source_knot_pk,target_knot_pk FROM adjacency_claims)"
        ).fetchone()[0],
    }
    db.commit()
    db.close()
    os.replace(temporary, args.output)
    elapsed = time.monotonic() - started
    output_sha256 = file_sha256(args.output)
    manifest = {
        "format": "unknotdb-brittenham-adjacency-import-v1",
        "parent": str(args.input),
        "parent_sha256": parent_sha256,
        "output": str(args.output),
        "output_sha256": output_sha256,
        "output_bytes": args.output.stat().st_size,
        "growth_bytes": args.output.stat().st_size - args.input.stat().st_size,
        "importer_sha256": builder_sha256,
        "retrieved_at": args.retrieved_at,
        "sources": {
            source_id: {
                "path": str(archive.resolve()),
                "sha256": expected_sha,
                "url": source_url,
                "stats": per_source[source_id],
            }
            for archive, expected_sha, source_id, source_url in sources
        },
        "identification_source_13": {
            "path": str(args.identifications_13_zip.resolve()),
            "sha256": identification_sha256,
            "url": "https://www.math.unl.edu/~mbrittenham2/unknottingsearch/database/13_crossing_knots.zip",
            "identified_dt_codes": len(identified_13) // 2,
        },
        "totals": totals,
        "elapsed_seconds": elapsed,
        "validation": {"integrity_check": integrity, "foreign_key_check_rows": 0},
    }
    manifest_temp = args.manifest.with_name(f"{args.manifest.name}.tmp-{os.getpid()}")
    manifest_temp.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    os.replace(manifest_temp, args.manifest)
    inserted = sum(stats["inserted"] for stats in per_source.values())
    unmapped = sum(stats.get("unmapped_target", 0) for stats in per_source.values())
    composite = sum(
        stats.get("composite_distance_one_skipped", 0) for stats in per_source.values()
    )
    report = f"""# Brittenham crossing-adjacency import v1

- Parent sidecar: `{args.input}` ({args.input.stat().st_size / 2**20:.2f} MiB)
- Output sidecar: `{args.output}` ({args.output.stat().st_size / 2**20:.2f} MiB)
- Growth: {(args.output.stat().st_size - args.input.stat().st_size) / 2**20:.2f} MiB
- SHA-256: `{output_sha256}`
- Imported `diagram_attested` claims: {inserted:,}
- Distinct named adjacency pairs (all trust levels): {totals["distinct_named_pairs"]:,}
- Unmapped prime targets skipped: {unmapped:,}
- Composite-target distance-one rows skipped: {composite:,}
- Existing `replay_verified` claims retained: {totals["replay_verified"]:,}
- Elapsed: {elapsed:.1f} seconds
- Integrity check: `{integrity}`; foreign-key violations: 0

Only Hamming-distance-one rows modulo a global mirror are imported.  The source
diagram, DT entry/crossing locator, raw successor DT, target identification and
archive line are retained.  These records remain `diagram_attested`; they do
not change proof-graph routes or `U_upper` until an independent planar replay is
compiled and validated.
"""
    report_temp = args.report.with_name(f"{args.report.name}.tmp-{os.getpid()}")
    report_temp.write_text(report)
    os.replace(report_temp, args.report)
    print(json.dumps({"sources": per_source, "totals": totals}, sort_keys=True))


if __name__ == "__main__":
    main()
