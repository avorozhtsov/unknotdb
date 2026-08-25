"""Extract braid words actually recorded by RF Knots into a durable corpus.

RF Knots is treated as read-only.  The output is deliberately a source corpus,
not an Unknot DB key set: canonicalisation, preprocessing and mirror quotienting
belong to the pinned Unknot DB contract and happen during ingestion.
"""

from __future__ import annotations

import argparse
import ast
import csv
import hashlib
import json
import subprocess
from collections import defaultdict
from pathlib import Path
from typing import Any

SCHEMA = "unknotdb-rf-representation-corpus-v1"
VERSION = "2026-08-26-v1"


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha256(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def representation_id(word: tuple[int, ...], strands: int) -> str:
    payload = json.dumps(
        {"encoding": "braid-word-v1", "strands": strands, "word": list(word)},
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
    return f"braid:{sha256_bytes(payload)}"


def component_count(word: tuple[int, ...], strands: int) -> int:
    permutation = list(range(strands))
    for letter in word:
        index = abs(letter) - 1
        permutation[index], permutation[index + 1] = (
            permutation[index + 1],
            permutation[index],
        )
    seen: set[int] = set()
    cycles = 0
    for start in range(strands):
        if start in seen:
            continue
        cycles += 1
        current = start
        while current not in seen:
            seen.add(current)
            current = permutation[current]
    return cycles


def validate(word_value: Any, strands_value: Any) -> tuple[tuple[int, ...], int]:
    if isinstance(strands_value, bool) or not isinstance(strands_value, int):
        raise TypeError("strands is not an integer")
    strands = int(strands_value)
    if strands < 1:
        raise ValueError("strands is less than one")
    if not isinstance(word_value, (list, tuple)):
        raise TypeError("word is not an array")
    word: list[int] = []
    for value in word_value:
        if isinstance(value, bool) or not isinstance(value, int):
            raise TypeError("word contains a non-integer")
        if value == 0:
            raise ValueError("word contains padding zero")
        if abs(value) >= strands:
            raise ValueError(f"generator {value} is outside B_{strands}")
        word.append(int(value))
    result = tuple(word)
    if component_count(result, strands) != 1:
        raise ValueError("closure is not a single-component knot")
    return result, strands


class Corpus:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.entries: dict[str, dict[str, Any]] = {}
        self.exclusions: list[dict[str, Any]] = []
        self.source_files: dict[str, dict[str, Any]] = {}

    def load_json(self, relative: str) -> Any:
        path = self.root / relative
        self.source_files[relative] = {
            "sha256": file_sha256(path),
            "bytes": path.stat().st_size,
        }
        return json.loads(path.read_text())

    def load_python(self, relative: str) -> ast.Module:
        path = self.root / relative
        self.source_files[relative] = {
            "sha256": file_sha256(path),
            "bytes": path.stat().st_size,
        }
        return ast.parse(path.read_text(), filename=relative)

    def add(
        self,
        word_value: Any,
        strands_value: Any,
        *,
        path: str,
        pointer: str,
        role: str,
        priority: int,
        source_id: str | None = None,
        instance_id: str | None = None,
        known_u: int | None = None,
    ) -> None:
        try:
            word, strands = validate(word_value, strands_value)
        except (TypeError, ValueError) as error:
            self.exclusions.append(
                {"path": path, "pointer": pointer, "role": role, "reason": str(error)}
            )
            return
        identity = representation_id(word, strands)
        entry = self.entries.setdefault(
            identity,
            {
                "representation_id": identity,
                "encoding": "braid-word-v1",
                "strands": strands,
                "word": list(word),
                "word_length": len(word),
                "priority": priority,
                "roles": [],
                "source_refs": [],
            },
        )
        entry["priority"] = min(entry["priority"], priority)
        if role not in entry["roles"]:
            entry["roles"].append(role)
        ref: dict[str, Any] = {"path": path, "pointer": pointer, "role": role}
        if source_id is not None:
            ref["source_id"] = str(source_id)
        if instance_id is not None:
            ref["instance_id"] = str(instance_id)
        if known_u is not None:
            ref["known_unknotting_number"] = int(known_u)
        if ref not in entry["source_refs"]:
            entry["source_refs"].append(ref)

    def extract_benchmarks(self) -> None:
        for relative, role, priority in (
            ("benchmarks/rungs-v1.json", "ladder-benchmark", 0),
            (
                "benchmarks/dkt2026-table1-authors-pd-braids-v1.json",
                "dkt-benchmark",
                0,
            ),
        ):
            data = self.load_json(relative)
            for index, row in enumerate(data["instances"]):
                payload = row.get("payload", {})
                self.add(
                    payload.get("word"),
                    payload.get("strands"),
                    path=relative,
                    pointer=f"/instances/{index}/payload",
                    role=role,
                    priority=priority,
                    source_id=row.get("source_id"),
                    instance_id=row.get("instance_id"),
                    known_u=row.get("known_unknotting_number"),
                )

    def extract_evidence_index(self) -> None:
        relative = "benchmarks/unknotting-evidence-index-20260815.json"
        data = self.load_json(relative)
        for knot_name, knot in sorted(data["knots"].items()):
            starts: dict[tuple[int, tuple[int, ...]], list[str]] = defaultdict(list)
            for scientist, evidence in sorted(knot.get("scientists", {}).items()):
                start = evidence.get("start")
                if not isinstance(start, dict):
                    continue
                try:
                    word, strands = validate(start.get("word"), start.get("strands"))
                except (TypeError, ValueError) as error:
                    self.exclusions.append(
                        {
                            "path": relative,
                            "pointer": f"/knots/{knot_name}/scientists/{scientist}/start",
                            "role": "replay-verified-evidence-start",
                            "reason": str(error),
                        }
                    )
                    continue
                starts[(strands, word)].append(scientist)
            for variant, ((strands, word), scientists) in enumerate(sorted(starts.items())):
                self.add(
                    word,
                    strands,
                    path=relative,
                    pointer=f"/knots/{knot_name}/scientists/*/start#variant={variant}",
                    role="replay-verified-evidence-start",
                    priority=0,
                    source_id=knot_name,
                )
                identity = representation_id(word, strands)
                self.entries[identity]["source_refs"][-1]["scientists"] = scientists
            if len(starts) > 1:
                self.exclusions.append(
                    {
                        "path": relative,
                        "pointer": f"/knots/{knot_name}/scientists",
                        "role": "evidence-start-consistency",
                        "reason": f"scientists record {len(starts)} distinct starts; all retained",
                    }
                )

    def extract_gap_candidates(self) -> None:
        relative = "benchmarks/knotinfo-unknotting-gap-candidates-20260814.json"
        data = self.load_json(relative)
        for index, row in enumerate(data["candidates"]):
            stored = row.get("stored_representation")
            if not isinstance(stored, dict):
                self.exclusions.append(
                    {
                        "path": relative,
                        "pointer": f"/candidates/{index}",
                        "role": "gap-candidate",
                        "source_id": row.get("canonical_name"),
                        "reason": f"no braid representation: {row.get('representation_status', 'unknown')}",
                    }
                )
                continue
            self.add(
                stored.get("word"),
                stored.get("strands"),
                path=relative,
                pointer=f"/candidates/{index}/stored_representation",
                role="gap-candidate",
                priority=1,
                source_id=row.get("canonical_name"),
                instance_id=stored.get("instance_id"),
            )

    def extract_scheduled(self) -> None:
        relative = "src/rf_knots/data/scheduled_unknotting_numbers.json"
        data = self.load_json(relative)
        for source_id, row in sorted(data["values"].items()):
            self.add(
                row.get("word"),
                row.get("strands"),
                path=relative,
                pointer=f"/values/{source_id}",
                role="scheduled-ladder-source",
                priority=0,
                source_id=source_id,
                known_u=row.get("unknotting_number"),
            )

    def extract_knot_table(self) -> None:
        relative = "src/rf_knots/data/knot_table.json"
        data = self.load_json(relative)
        for name, row in sorted(data["knots"].items()):
            self.add(
                row.get("braid"),
                row.get("strands"),
                path=relative,
                pointer=f"/knots/{name}",
                role="bundled-knot-table",
                priority=2,
                source_id=name,
            )
        for index, row in enumerate(data.get("skipped", [])):
            self.exclusions.append(
                {
                    "path": relative,
                    "pointer": f"/skipped/{index}",
                    "role": "bundled-knot-table",
                    "source_id": row.get("name"),
                    "reason": "no stored braid; builder strand cap exceeded",
                }
            )

    @staticmethod
    def _fixture_literal(node: ast.AST) -> Any:
        if isinstance(node, ast.Constant) and isinstance(node.value, (int, str)):
            return node.value
        if isinstance(node, ast.Tuple):
            return tuple(Corpus._fixture_literal(value) for value in node.elts)
        if isinstance(node, ast.List):
            return [Corpus._fixture_literal(value) for value in node.elts]
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            value = Corpus._fixture_literal(node.operand)
            if isinstance(value, int):
                return -value
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Mult):
            left = Corpus._fixture_literal(node.left)
            right = Corpus._fixture_literal(node.right)
            if isinstance(left, (tuple, list)) and isinstance(right, int):
                return left * right
            if isinstance(left, int) and isinstance(right, (tuple, list)):
                return right * left
        raise ValueError(f"unsupported fixture expression {ast.dump(node, include_attributes=False)}")

    def extract_test_fixtures(self) -> None:
        specs = (
            ("tests/test_unknot_search.py", "KNOWN", 0, 1),
            ("tests/test_certified_value.py", "KNOWN", 0, 1),
            ("tests/test_torus.py", "WORDS", 0, 1),
            ("tests/test_seifert.py", "CLASSICAL", 1, 2),
        )
        for relative, variable, word_index, strands_index in specs:
            tree = self.load_python(relative)
            value_node: ast.AST | None = None
            for node in tree.body:
                if isinstance(node, ast.Assign) and any(
                    isinstance(target, ast.Name) and target.id == variable
                    for target in node.targets
                ):
                    value_node = node.value
                if (
                    isinstance(node, ast.AnnAssign)
                    and isinstance(node.target, ast.Name)
                    and node.target.id == variable
                ):
                    value_node = node.value
            if value_node is None:
                self.exclusions.append(
                    {
                        "path": relative,
                        "pointer": variable,
                        "role": "test-fixture",
                        "reason": "named fixture assignment not found",
                    }
                )
                continue
            try:
                rows = self._fixture_literal(value_node)
            except ValueError as error:
                self.exclusions.append(
                    {
                        "path": relative,
                        "pointer": variable,
                        "role": "test-fixture",
                        "reason": str(error),
                    }
                )
                continue
            for index, row in enumerate(rows):
                self.add(
                    row[word_index],
                    row[strands_index],
                    path=relative,
                    pointer=f"{variable}[{index}]",
                    role="test-fixture",
                    priority=1,
                )

    def finish(self, output: Path, tsv_output: Path | None = None) -> None:
        entries = sorted(
            self.entries.values(),
            key=lambda row: (row["priority"], row["strands"], row["word_length"], row["representation_id"]),
        )
        for entry in entries:
            entry["roles"].sort()
            entry["source_refs"].sort(
                key=lambda ref: (ref["role"], ref["path"], ref["pointer"])
            )
        by_priority: dict[str, int] = defaultdict(int)
        by_role: dict[str, int] = defaultdict(int)
        for entry in entries:
            by_priority[str(entry["priority"])] += 1
            for role in entry["roles"]:
                by_role[role] += 1
        payload = {
            "schema": SCHEMA,
            "version": VERSION,
            "source_repository": "RF Knots (read-only input)",
            "source_git_head": subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=self.root,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip(),
            "source_files": dict(sorted(self.source_files.items())),
            "contract": {
                "input_encoding": "braid-word-v1",
                "position_semantics": "cyclic",
                "canonicalization": "deferred to pinned Unknot DB ingestion contract",
                "mirror_quotient": "deferred to pinned Unknot DB ingestion contract",
                "validation": "nonzero in-range generators and single-component closure",
            },
            "priority_semantics": {
                "0": "actual ladder, DKT, scheduled, or replay-evidence target",
                "1": "explicit gap-analysis target with stored representation",
                "2": "bundled RF Knots catalogue representation",
            },
            "summary": {
                "unique_representations": len(entries),
                "by_priority": dict(sorted(by_priority.items())),
                "by_role": dict(sorted(by_role.items())),
                "exclusions": len(self.exclusions),
            },
            "entries": entries,
            "exclusions": sorted(
                self.exclusions,
                key=lambda row: (row["path"], row["pointer"], row["reason"]),
            ),
        }
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
        if tsv_output is not None:
            tsv_output.parent.mkdir(parents=True, exist_ok=True)
            with tsv_output.open("w", newline="") as stream:
                writer = csv.writer(stream, delimiter="\t", lineterminator="\n")
                writer.writerow(("representation_id", "priority", "strands", "word", "roles"))
                for entry in entries:
                    writer.writerow(
                        (
                            entry["representation_id"],
                            entry["priority"],
                            entry["strands"],
                            ",".join(str(letter) for letter in entry["word"]),
                            ",".join(entry["roles"]),
                        )
                    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rf-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--tsv-output", type=Path)
    args = parser.parse_args()
    corpus = Corpus(args.rf_root.resolve())
    corpus.extract_benchmarks()
    corpus.extract_evidence_index()
    corpus.extract_gap_candidates()
    corpus.extract_scheduled()
    corpus.extract_knot_table()
    corpus.extract_test_fixtures()
    corpus.finish(args.output, args.tsv_output)


if __name__ == "__main__":
    main()
