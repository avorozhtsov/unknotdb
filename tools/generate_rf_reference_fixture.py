"""Generate differential action vectors from an RF Knots checkout.

The output is deliberately a trivial line format so the Rust runtime test needs
no JSON/YAML dependency. Generation may depend on RF Knots; verification does
not. The source checkout is read-only.
"""

from __future__ import annotations

import argparse
import hashlib
import random
import struct
import subprocess
from pathlib import Path

from rf_knots import reference
from rf_knots.actions import INSERT, ActionSpec


def encode_representation(word: tuple[int, ...], strands: int, cyclic: bool) -> bytes:
    flags = int(cyclic)
    header = struct.pack("<4sBBHI", b"UKB0", 0, flags, strands, len(word))
    return header + struct.pack(f"<{len(word)}h", *word)


def semantic_action(spec: ActionSpec, flat: int) -> int:
    kind, position, generator, sign = spec.decode(flat)
    negative = kind == INSERT and sign < 0
    return kind | (position << 4) | (generator << 36 if kind == INSERT else 0) | (
        int(negative) << 52
    )


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rf-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=256)
    args = parser.parse_args()

    rf_root = args.rf_root.resolve()
    head = subprocess.check_output(
        ["git", "-C", str(rf_root), "rev-parse", "HEAD"], text=True
    ).strip()
    reference_path = rf_root / "src/rf_knots/reference.py"
    actions_path = rf_root / "src/rf_knots/actions.py"
    rng = random.Random(20260824)
    rows: dict[tuple[bytes, int], bytes] = {}

    for cyclic in (False, True):
        spec = ActionSpec(max_len=32, max_strands=8, cyclic_band_generators=cyclic)
        for strands in range(2, 7):
            largest = strands if cyclic and strands >= 3 else strands - 1
            seeds: list[tuple[int, ...]] = [(), (strands - 1,), (1, -1)]
            if strands >= 3:
                seeds.append((1, 2, 1))
            if strands >= 4:
                seeds.append((1, 3))
            if cyclic and strands >= 3:
                seeds.append((strands, 1, strands))
            seeds.extend(
                tuple(
                    rng.choice((-1, 1)) * rng.randint(1, largest)
                    for _ in range(rng.randint(1, 9))
                )
                for _ in range(90)
            )
            for word in seeds:
                for flat, target_word, target_strands in reference.successors(
                    spec, word, strands, allow_crossing=True
                ):
                    action = semantic_action(spec, flat)
                    source = encode_representation(word, strands, cyclic)
                    target = encode_representation(target_word, target_strands, cyclic)
                    rows[(source, action)] = target

    candidates = sorted(rows.items(), key=lambda item: (item[0][0], item[0][1]))
    by_kind: dict[int, list[tuple[tuple[bytes, int], bytes]]] = {}
    for row in candidates:
        by_kind.setdefault(row[0][1] & 0xF, []).append(row)
    selected: list[tuple[tuple[bytes, int], bytes]] = []
    for kind in sorted(by_kind):
        rng.shuffle(by_kind[kind])
        selected.extend(by_kind[kind][:16])
    selected_keys = {key for key, _ in selected}
    remainder = [row for row in candidates if row[0] not in selected_keys]
    rng.shuffle(remainder)
    selected.extend(remainder[: max(args.cases - len(selected), 0)])
    selected = selected[: args.cases]
    lines = [
        "# unknotdb RF-reference differential fixture v0",
        f"# rf_head={head}",
        f"# reference_py_sha256={file_sha256(reference_path)}",
        f"# actions_py_sha256={file_sha256(actions_path)}",
        "# source_representation_hex semantic_action_u63 target_representation_hex",
    ]
    lines.extend(
        f"{source.hex()} {action} {target.hex()}"
        for ((source, action), target) in selected
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
