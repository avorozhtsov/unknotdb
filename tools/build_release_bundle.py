#!/usr/bin/env python3
"""Assemble one graph-pinned, checksum-addressed Unknot DB release bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sqlite3
from pathlib import Path


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def metadata(path: Path) -> dict[str, str]:
    with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as connection:
        integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
        foreign_keys = connection.execute("PRAGMA foreign_key_check").fetchall()
        if integrity != "ok" or foreign_keys:
            raise ValueError(
                f"invalid SQLite component {path}: {integrity}, fk={len(foreign_keys)}"
            )
        return dict(connection.execute("SELECT key,value FROM meta"))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--identification", type=Path, required=True)
    parser.add_argument("--lookup", type=Path, required=True)
    parser.add_argument("--fingerprints", type=Path, required=True)
    parser.add_argument("--federation", type=Path, required=True)
    parser.add_argument("--provenance", type=Path, required=True)
    parser.add_argument(
        "--artifact",
        type=Path,
        action="append",
        default=[],
        help="small release report or manifest copied under reports/",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() and any(args.output.iterdir()):
        raise FileExistsError(f"release directory is not empty: {args.output}")

    inputs = {
        "proof.sqlite": args.graph,
        "identification.sqlite": args.identification,
        "lookup.sqlite": args.lookup,
        "fingerprints.sqlite": args.fingerprints,
        "federation.sqlite": args.federation,
        "provenance.sqlite": args.provenance,
    }
    metas = {name: metadata(path) for name, path in inputs.items()}
    hashes = {name: file_sha256(path) for name, path in inputs.items()}
    graph_hash = hashes["proof.sqlite"]
    requirements = {
        "identification.sqlite": {
            "graph_snapshot_sha256": graph_hash,
            "lookup_maps_sha256": hashes["lookup.sqlite"],
        },
        "lookup.sqlite": {"graph_snapshot_sha256": graph_hash},
        "fingerprints.sqlite": {
            "graph_snapshot_sha256": graph_hash,
            "lookup_maps_sha256": hashes["lookup.sqlite"],
            "identification_maps_sha256": hashes["identification.sqlite"],
        },
        "federation.sqlite": {
            "graph_snapshot_sha256": graph_hash,
            "identification_sidecar_sha256": hashes["identification.sqlite"],
        },
        "provenance.sqlite": {
            "graph_snapshot_sha256": graph_hash,
            "identification_sidecar_sha256": hashes["identification.sqlite"],
            "federation_sidecar_sha256": hashes["federation.sqlite"],
        },
    }
    for name, expected in requirements.items():
        for key, value in expected.items():
            if metas[name].get(key) != value:
                raise ValueError(
                    f"{name} metadata mismatch for {key}: "
                    f"{metas[name].get(key)} != {value}"
                )

    parent = args.output.parent
    parent.mkdir(parents=True, exist_ok=True)
    temporary = parent / f".{args.output.name}.{os.getpid()}.part"
    if temporary.exists():
        shutil.rmtree(temporary)
    temporary.mkdir()
    try:
        components = []
        for name, source in inputs.items():
            target = temporary / name
            shutil.copyfile(source, target)
            copied_hash = file_sha256(target)
            if copied_hash != hashes[name]:
                raise ValueError(f"copy hash mismatch for {name}")
            components.append(
                {
                    "name": name,
                    "sha256": copied_hash,
                    "bytes": target.stat().st_size,
                    "schema": metas[name].get("schema", "proof-graph-schema-v4"),
                }
            )
        report_components = []
        if args.artifact:
            reports = temporary / "reports"
            reports.mkdir()
            seen_names = set()
            for source in args.artifact:
                if source.name in seen_names:
                    raise ValueError(f"duplicate release artifact name: {source.name}")
                seen_names.add(source.name)
                target = reports / source.name
                shutil.copyfile(source, target)
                report_components.append(
                    {
                        "name": f"reports/{source.name}",
                        "sha256": file_sha256(target),
                        "bytes": target.stat().st_size,
                    }
                )
        manifest = {
            "format": "unknotdb-release-bundle-v1",
            "version": args.version,
            "proof_graph_sha256": graph_hash,
            "compatibility": "all graph-dependent sidecars hash-pin proof.sqlite",
            "components": components,
            "artifacts": report_components,
            "total_bytes": sum(item["bytes"] for item in components)
            + sum(item["bytes"] for item in report_components),
        }
        (temporary / "release.json").write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n"
        )
        checksum_lines = [
            f"{item['sha256']}  {item['name']}"
            for item in components + report_components
        ]
        checksum_lines.append(
            f"{file_sha256(temporary / 'release.json')}  release.json"
        )
        (temporary / "SHA256SUMS").write_text("\n".join(checksum_lines) + "\n")
        (temporary / "README.md").write_text(
            f"# Unknot DB {args.version}\n\n"
            "This is a coherent pre-release bundle. `proof.sqlite` is the only "
            "proof authority; every other SQLite file is a graph-pinned metadata "
            "sidecar and cannot change replay-validated U upper bounds. Verify "
            "`SHA256SUMS` before use.\n"
        )
        if args.output.exists():
            args.output.rmdir()
        os.replace(temporary, args.output)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    print(json.dumps(manifest, sort_keys=True))


if __name__ == "__main__":
    main()
