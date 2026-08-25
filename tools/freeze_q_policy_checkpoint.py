"""Freeze one inference-only scientist from an immutable read of a Q state.

The source state may be a live atomically-replaced ``state.pt.gz``. This tool
opens it once, hashes exactly those bytes, loads from that in-memory snapshot,
and writes a self-contained network checkpoint plus a JSON provenance manifest.
It never trains or mutates the source experiment.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


def sha256_bytes(blob: bytes) -> str:
    return hashlib.sha256(blob).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_torch_save(torch: Any, payload: dict[str, Any], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
    try:
        torch.save(payload, temporary)
        with temporary.open("rb") as stream:
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_json(payload: dict[str, Any], path: Path) -> None:
    encoded = (json.dumps(payload, indent=2, sort_keys=True) + "\n").encode()
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(encoded)
        stream.flush()
        os.fsync(stream.fileno())
    try:
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pgx-root", type=Path, required=True)
    parser.add_argument("--source-q-state", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--scientist", default="raster-axial-12")
    parser.add_argument("--lineage", default="q-grown-raster-axial-12")
    parser.add_argument("--q-generation", default="Q254")
    args = parser.parse_args()

    sys.path.insert(0, str((args.pgx_root / "src").resolve()))
    import torch  # type: ignore[import-not-found]

    source = args.source_q_state.resolve()
    source_blob = source.read_bytes()
    source_sha256 = sha256_bytes(source_blob)
    with gzip.GzipFile(fileobj=io.BytesIO(source_blob), mode="rb") as stream:
        q_state = torch.load(stream, map_location="cpu", weights_only=False)

    scientist_state = q_state["scientists"][args.scientist]
    network = {
        key: value.detach().cpu().clone()
        for key, value in scientist_state["network"].items()
    }
    frozen_at = datetime.now(UTC).replace(microsecond=0).isoformat()
    processed = [str(item) for item in q_state.get("processed", [])]
    processed_sha256 = sha256_bytes(
        json.dumps(processed, separators=(",", ":")).encode()
    )
    checkpoint_payload = {
        "schema": "unknotdb-frozen-policy-checkpoint-v0",
        "scientist": args.scientist,
        "lineage": args.lineage,
        "q_generation": args.q_generation,
        "network": network,
        "prediction_source": scientist_state.get("prediction_source", "unknown"),
        "source_q_state_sha256": source_sha256,
        "processed_count": len(processed),
        "processed_order_sha256": processed_sha256,
        "frozen_at": frozen_at,
    }
    checkpoint = args.output_dir.resolve() / "checkpoint.pt"
    atomic_torch_save(torch, checkpoint_payload, checkpoint)
    checkpoint_sha256 = sha256_file(checkpoint)
    model_id = f"{args.lineage}:{args.q_generation}:{checkpoint_sha256}"

    manifest = {
        "schema": "unknotdb-frozen-policy-manifest-v0",
        "model_id": model_id,
        "scientist": args.scientist,
        "lineage": args.lineage,
        "q_generation": args.q_generation,
        "objective_ratio": 1000,
        "controller_initial_state": "canonical-clean-v0",
        "checkpoint": "checkpoint.pt",
        "checkpoint_sha256": checkpoint_sha256,
        "checkpoint_bytes": checkpoint.stat().st_size,
        "source_q_state": str(source),
        "source_q_state_sha256": source_sha256,
        "source_snapshot_semantics": "single-open-read-of-possibly-live-atomic-state",
        "processed_count": len(processed),
        "processed_order_sha256": processed_sha256,
        "frozen_at": frozen_at,
        "training_performed": False,
    }
    atomic_json(manifest, args.output_dir.resolve() / "manifest.json")
    print(json.dumps(manifest, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
