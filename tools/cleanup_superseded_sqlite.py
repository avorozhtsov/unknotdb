#!/usr/bin/env python3
"""Delete only audited generated SQLite artifacts after a release gate passes."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

PATTERNS = ("*.sqlite", "*.sqlite.tmp-*", "*.sqlite-wal", "*.sqlite-shm")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    release = args.release.resolve()
    archive = args.archive.resolve()
    manifest = args.manifest.resolve()
    if not (release / "SHA256SUMS").is_file() or not archive.is_file():
        raise ValueError("validated release directory and archive are required")
    if release.is_relative_to(root / "outputs") or release.is_relative_to(root / "tmp"):
        raise ValueError("release must be outside cleanup roots")
    if manifest.exists():
        raise FileExistsError(manifest)

    targets = set()
    for directory in (root / "outputs", root / "tmp"):
        for pattern in PATTERNS:
            targets.update(path.resolve() for path in directory.rglob(pattern))
    targets = sorted(path for path in targets if path.is_file())
    allowed_roots = (root / "outputs", root / "tmp")
    if any(
        not any(path.is_relative_to(base) for base in allowed_roots) for path in targets
    ):
        raise ValueError("cleanup target escaped the exact generated-data roots")
    rows = [
        {
            "path": str(path.relative_to(root)),
            "bytes": path.stat().st_size,
            "sha256": file_sha256(path),
        }
        for path in targets
    ]
    document = {
        "format": "unknotdb-superseded-sqlite-cleanup-v1",
        "release": str(release.relative_to(root)),
        "release_manifest_sha256": file_sha256(release / "release.json"),
        "archive": str(archive.relative_to(root)),
        "archive_sha256": file_sha256(archive),
        "executed": args.execute,
        "recoverable_from_local_files": False,
        "target_count": len(rows),
        "target_bytes": sum(row["bytes"] for row in rows),
        "targets": rows,
    }
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")
    if args.execute:
        for path in targets:
            path.unlink()
    print(
        f"executed={args.execute} targets={len(rows)} bytes={document['target_bytes']} "
        f"manifest={manifest}"
    )


if __name__ == "__main__":
    main()
