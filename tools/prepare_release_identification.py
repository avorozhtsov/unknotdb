#!/usr/bin/env python3
"""Atomically repin an identification sidecar to release lookup maps."""

from __future__ import annotations

import argparse
import hashlib
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--lookup-maps", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--release-version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)

    graph_sha256 = file_sha256(args.graph)
    lookup_sha256 = file_sha256(args.lookup_maps)
    with sqlite3.connect(f"file:{args.lookup_maps}?mode=ro", uri=True) as lookup:
        lookup_meta = dict(lookup.execute("SELECT key,value FROM meta"))
    if lookup_meta.get("graph_snapshot_sha256") != graph_sha256:
        raise ValueError("lookup maps do not pin the supplied graph")

    temporary = args.output.with_name(f"{args.output.name}.tmp-{os.getpid()}")
    shutil.copyfile(args.input, temporary)
    try:
        with sqlite3.connect(temporary) as connection:
            metadata = dict(connection.execute("SELECT key,value FROM meta"))
            if metadata.get("graph_snapshot_sha256") != graph_sha256:
                raise ValueError(
                    "identification sidecar does not pin the supplied graph"
                )
            connection.executemany(
                "INSERT OR REPLACE INTO meta(key,value) VALUES (?,?)",
                (
                    ("lookup_maps_sha256", lookup_sha256),
                    ("release_version", args.release_version),
                    ("release_parent_identification_sha256", file_sha256(args.input)),
                ),
            )
            connection.execute("PRAGMA optimize")
            integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
            foreign_keys = connection.execute("PRAGMA foreign_key_check").fetchall()
            if integrity != "ok" or foreign_keys:
                raise ValueError(
                    f"identification validation failed: integrity={integrity}, "
                    f"foreign_keys={len(foreign_keys)}"
                )
        os.replace(temporary, args.output)
    finally:
        temporary.unlink(missing_ok=True)

    print(
        f"output={args.output} sha256={file_sha256(args.output)} "
        f"lookup_maps_sha256={lookup_sha256} graph_sha256={graph_sha256}"
    )


if __name__ == "__main__":
    main()
