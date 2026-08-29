#!/usr/bin/env python3
"""Run frozen Q254 MCTS with exact UnknotDB keys as terminal oracles.

The graph can terminate search, but it never certifies a new edge here.  This
runner emits exact semantic-action/state traces for a later Rust replay/import
gate.  The network is loaded inference-only and no optimizer step is executed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import sqlite3
import struct
import sys
import time
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any

import numpy as np
import torch


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def atomic_json(path: Path, payload: object) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.part")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, path)


def append_jsonl(path: Path, payload: object) -> None:
    with path.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def minimal_rotation(word: list[int]) -> list[int]:
    if not word:
        return []
    return list(min(tuple(word[offset:] + word[:offset]) for offset in range(len(word))))


def normalize_with_witness(word: list[int]) -> tuple[list[int], dict[str, object]]:
    writhe = sum(word)
    if writhe > 0:
        mirrored = False
        base = word
    elif writhe < 0:
        mirrored = True
        base = [-letter for letter in word]
    else:
        original = minimal_rotation(word)
        reflected = minimal_rotation([-letter for letter in word])
        mirrored = reflected < original
        base = [-letter for letter in word] if mirrored else word
    canonical = minimal_rotation(base)
    shift = next(
        (offset for offset in range(len(base)) if base[offset:] + base[:offset] == canonical),
        0,
    )
    return canonical, {"kind": "normalize", "mirrored": mirrored, "rotate_left": shift}


def normalized(word: list[int], strands: int) -> tuple[list[int], bytes]:
    canonical, _ = normalize_with_witness(word)
    encoded = b"UKB0" + struct.pack("<BBHI", 0, 0, strands, len(canonical))
    encoded += struct.pack(f"<{len(canonical)}h", *canonical) if canonical else b""
    return canonical, hashlib.sha256(encoded).digest()


def reduce_to_key(word: list[int], strands: int) -> tuple[bytes, list[dict[str, object]]]:
    current, initial = normalize_with_witness(word)
    program = [initial]
    while True:
        top = strands - 1
        top_positions = [index for index, letter in enumerate(current) if abs(letter) == top]
        if strands >= 2 and len(top_positions) == 1:
            position = top_positions[0]
            current.pop(position)
            strands -= 1
            program.append({"kind": "destabilize"})
            continue
        reduction = next(
            (
                position
                for position in range(len(current))
                if len(current) >= 2 and current[position] == -current[(position + 1) % len(current)]
            ),
            None,
        )
        if reduction is None:
            break
        right = (reduction + 1) % len(current)
        for index in sorted((reduction, right), reverse=True):
            current.pop(index)
        program.append({"kind": "reduce", "position": reduction})
    current, final = normalize_with_witness(current)
    program.append(final)
    _, key = normalized(current, strands)
    return key, program


def braid_of(game: Any, state: Any) -> tuple[list[int], int]:
    raw = game.unwrap(state)
    word = [int(value) for value in np.asarray(raw._word) if int(value)]
    return word, int(np.asarray(raw._n))


@dataclass(frozen=True)
class GraphOracleState:
    base_state: Any
    hit_key: str = ""
    target_u: int = -1
    graph_hit: bool = False
    terminal_program: tuple[dict[str, object], ...] = ()


class GraphTerminalGame:
    """Exact-transition MCTS wrapper with canonical graph membership terminals."""

    def __init__(self, base: Any, knot: Any, graph_u: dict[bytes, int], target_upper: int):
        self.base = base
        self.config = base.config
        self.knot = knot
        self.graph_u = graph_u
        self.target_upper = target_upper
        self.ratio = 1000.0
        self.objective_cap = None

    def _wrap(self, transition: Any, *, allow_hit: bool) -> Any:
        word, strands = braid_of(self.base, transition.state)
        _, direct_key = normalized(word, strands)
        reduced_key, terminal_program = reduce_to_key(word, strands)
        raw = self.base.unwrap(transition.state)
        used_cc = int(np.asarray(raw._crossing_changes))
        direct_u = self.graph_u.get(direct_key)
        reduced_u = self.graph_u.get(reduced_key)
        if direct_u is not None and (reduced_u is None or direct_u <= reduced_u):
            key, target_u, suffix = direct_key, direct_u, ()
        else:
            key, target_u, suffix = reduced_key, reduced_u, tuple(terminal_program)
        useful = allow_hit and target_u is not None and used_cc + target_u <= self.target_upper
        state = GraphOracleState(
            transition.state,
            key.hex() if useful else "",
            int(target_u) if useful else -1,
            useful,
            suffix if useful else (),
        )
        if not useful:
            return replace(transition, state=state)
        semantic_moves = self.base.semantic_move_count(transition.state)
        worst = 1001.0 * max(int(self.config.simplify_budget), 1)
        objective = 1000.0 * (used_cc + int(target_u)) + semantic_moves
        payoff = 1.0 - 2.0 * min(max(objective / worst, 0.0), 1.0)
        return replace(
            transition,
            state=state,
            reward=payoff,
            terminated=True,
            termination_reason="unknotdb_graph_hit",
        )

    def reset(self, seed: int) -> Any:
        del seed
        transition = self.base.from_word(
            list(self.knot.word), self.knot.strands, log_ratio=math.log(self.ratio)
        )
        return self._wrap(transition, allow_hit=False)

    def step(self, state: GraphOracleState, action: int) -> Any:
        if state.graph_hit:
            raise ValueError("cannot step an UnknotDB graph terminal")
        return self._wrap(self.base.step(state.base_state, action), allow_hit=True)

    def final_rewards(self, state: GraphOracleState) -> np.ndarray:
        if state.graph_hit:
            raw = self.base.unwrap(state.base_state)
            simplifier = 1 - int(np.asarray(raw._scrambler))
            rewards = np.zeros(2, dtype=np.float32)
            rewards[simplifier] = 1.0
            rewards[1 - simplifier] = -1.0
            return rewards
        return self.base.final_rewards(state.base_state)

    def value_potential(self, state: GraphOracleState, player: int) -> float:
        return self.base.value_potential(state.base_state, player)

    def state_info(self, state: GraphOracleState) -> dict[str, int]:
        return self.base.state_info(state.base_state)

    def first_role_player(self, state: GraphOracleState) -> int:
        return self.base.first_role_player(state.base_state)

    def unwrap(self, state: GraphOracleState) -> Any:
        return self.base.unwrap(state.base_state)

    def semantic_move_count(self, state: GraphOracleState) -> int:
        return self.base.semantic_move_count(state.base_state)

    def native_ply_count(self, state: GraphOracleState) -> int:
        return self.base.native_ply_count(state.base_state)

    def internal_ply_count(self, state: GraphOracleState) -> int:
        return self.base.internal_ply_count(state.base_state)


def load_graph_u(path: Path) -> dict[bytes, int]:
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    rows = db.execute(
        "SELECT k.rep_key,n.u_upper_bound FROM node_keys k JOIN nodes n USING(node_id) "
        "WHERE k.key_kind=0 AND n.u_upper_bound IS NOT NULL"
    )
    result = {bytes(key): int(value) for key, value in rows}
    db.close()
    return result


def raw_checkpoint(game: Any, state: GraphOracleState) -> dict[str, object]:
    word, strands = braid_of(game, state)
    return {"strands": strands, "word": word}


def solve_one(
    scientist: Any,
    graph_u: dict[bytes, int],
    row: dict[str, Any],
    simulations: int,
    attempts: int,
    seed: int,
) -> dict[str, Any]:
    from pgx_mcts_bench.adaptive_scientists import KnotItem
    from pgx_mcts_bench.search import NeuralMCTS
    knot = KnotItem(
        row["name"], int(row["crossing_number"]), tuple(row["word"]), int(row["strands"])
    )
    game = GraphTerminalGame(scientist.game, knot, graph_u, int(row["target_upper"]))
    search = NeuralMCTS(
        game,
        scientist.network,
        replace(scientist.config.search, simulations=simulations),
        "cpu",
    )
    attempt_rows = []
    best = None
    for attempt in range(attempts):
        episode_seed = seed + attempt
        rng = np.random.default_rng(episode_seed)
        transition = game.reset(episode_seed)
        actions = []
        checkpoints = []
        started = time.perf_counter()
        while not transition.terminated:
            result = search.run(
                transition.state,
                transition.observation,
                transition.legal_actions,
                rng,
                temperature=0.0,
                add_root_noise=attempt > 0,
            )
            state = transition.state.base_state
            length = int(np.count_nonzero(np.asarray(state.pgx._word)))
            underlying = scientist.game.underlying_action(int(result.action), state.head, length)
            transition = game.step(transition.state, int(result.action))
            if underlying is not None:
                kind, position, generator, sign = scientist.game.spec.decode(int(underlying))
                actions.append(
                    {
                        "flat": int(underlying),
                        "kind": int(kind),
                        "position": int(position),
                        "generator": int(generator),
                        "sign": int(sign),
                    }
                )
                checkpoints.append(raw_checkpoint(game, transition.state))
        elapsed = time.perf_counter() - started
        raw = game.unwrap(transition.state)
        used_cc = int(np.asarray(raw._crossing_changes))
        graph_hit = transition.state.graph_hit
        total_u = used_cc + transition.state.target_u if graph_hit else None
        attempt_row = {
            "attempt": attempt,
            "seed": episode_seed,
            "graph_hit": graph_hit,
            "target_key": transition.state.hit_key or None,
            "target_graph_u": transition.state.target_u if graph_hit else None,
            "terminal_program": list(transition.state.terminal_program) if graph_hit else [],
            "route_cc": used_cc,
            "total_u": total_u,
            "semantic_moves": len(actions),
            "native_plies": game.native_ply_count(transition.state),
            "elapsed_seconds": elapsed,
            "termination_reason": transition.termination_reason,
            "actions": actions if graph_hit else [],
            "checkpoints": checkpoints if graph_hit else [],
        }
        attempt_rows.append(attempt_row)
        if graph_hit and (best is None or (total_u, len(actions)) < (best["total_u"], best["semantic_moves"])):
            best = attempt_row
        if best is not None and int(best["total_u"]) <= int(row["u_lower"]):
            break
    return {
        "knot_id": row["knot_id"],
        "target_upper": row["target_upper"],
        "u_lower": row["u_lower"],
        "simulations_per_native_ply": simulations,
        "attempt_budget": attempts,
        "success": best is not None,
        "best": best,
        "attempts": attempt_rows,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cohort", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--pgx-root", type=Path, required=True)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--simulations", type=int, default=64)
    parser.add_argument("--attempts", type=int, default=2)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--skip", type=int, default=0)
    parser.add_argument("--seed", type=int, default=20260828)
    args = parser.parse_args()
    if args.simulations <= 0 or args.attempts <= 0 or args.skip < 0:
        raise ValueError("budgets must be positive and skip non-negative")
    manifest = json.loads((args.model_dir / "manifest.json").read_text())
    checkpoint = args.model_dir / manifest["checkpoint"]
    if file_sha256(checkpoint) != manifest["checkpoint_sha256"]:
        raise ValueError("frozen Q254 checkpoint hash mismatch")
    if int(manifest["objective_ratio"]) != 1000:
        raise ValueError("checkpoint is not pinned to L1000")

    sys.path.insert(0, str((args.pgx_root / "src").resolve()))
    from pgx_mcts_bench.adaptive_scientists import load_scientist

    torch.set_num_threads(1)
    scientist = load_scientist(
        manifest["scientist"], checkpoint, seed=args.seed, device="cpu",
        simulations=args.simulations, require_factorized=True, objective_budget_channel=True,
    )
    scientist.network.eval()
    graph_u = load_graph_u(args.graph)
    cohort = json.loads(args.cohort.read_text())
    selected = [row for row in cohort["rows"] if row["q254_compatible"]][args.skip :]
    if args.limit is not None:
        selected = selected[: args.limit]

    args.output_dir.mkdir(parents=True, exist_ok=True)
    events = args.output_dir / "events.jsonl"
    run_manifest = args.output_dir / "manifest.json"
    results = []
    started = time.monotonic()
    for index, row in enumerate(selected, 1):
        item_path = args.output_dir / "items" / f"{row['name']}.json"
        item_path.parent.mkdir(parents=True, exist_ok=True)
        if item_path.exists():
            result = json.loads(item_path.read_text())
            status = "resume"
        else:
            result = solve_one(
                scientist, graph_u, row, args.simulations, args.attempts,
                args.seed + (args.skip + index) * 1_000_000,
            )
            atomic_json(item_path, result)
            append_jsonl(events, {"index": index, **{k: result[k] for k in ("knot_id", "success")}})
            status = "solved" if result["success"] else "miss"
        results.append(result)
        print(f"{index}/{len(selected)}\t{row['knot_id']}\t{status}", flush=True)
        atomic_json(
            run_manifest,
            {
                "schema": "unknotdb-dgkt-q254-graph-mcts-run-v0",
                "status": "running",
                "model_id": manifest["model_id"],
                "checkpoint_sha256": manifest["checkpoint_sha256"],
                "training_performed": False,
                "graph": str(args.graph.resolve()),
                "graph_sha256": file_sha256(args.graph),
                "cohort": str(args.cohort.resolve()),
                "cohort_sha256": file_sha256(args.cohort),
                "budgets": {"simulations_per_native_ply": args.simulations, "attempts": args.attempts},
                "range": {"skip": args.skip, "selected": len(selected), "completed": len(results)},
                "summary": {"successes": sum(row["success"] for row in results)},
                "elapsed_seconds": time.monotonic() - started,
            },
        )
    final = json.loads(run_manifest.read_text()) if run_manifest.exists() else {}
    final["status"] = "complete"
    final["elapsed_seconds"] = time.monotonic() - started
    atomic_json(run_manifest, final)


if __name__ == "__main__":
    main()
