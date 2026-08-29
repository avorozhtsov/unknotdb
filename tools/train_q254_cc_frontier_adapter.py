#!/usr/bin/env python3
"""Compile set-valued proof targets and train an isolated frozen-Q254 adapter.

The graph supplies only replayed comparisons.  Missing actions stay unknown.
For serial controllers, a semantic CC may be preceded by at most five internal
head/tape actions; longer native routes are excluded rather than truncated.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import sqlite3
import struct
import time
from collections import defaultdict
from pathlib import Path

import numpy as np
import torch

SCHEMA = "unknotdb-q254-cc-frontier-adapter-v0"


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


def decode_ukb0(encoded: bytes) -> tuple[list[int], int, bool]:
    if len(encoded) < 12 or encoded[:4] != b"UKB0" or encoded[4] != 0:
        raise ValueError("unsupported UKB0 representation")
    flags = encoded[5]
    strands = struct.unpack_from("<H", encoded, 6)[0]
    length = struct.unpack_from("<I", encoded, 8)[0]
    if len(encoded) != 12 + 2 * length:
        raise ValueError("UKB0 word length mismatch")
    word = list(struct.unpack_from(f"<{length}h", encoded, 12))
    return word, strands, bool(flags & 1)


def semantic_cc_position(action_u63: int) -> int:
    if action_u63 & 0xF != 8 or action_u63 >> 53:
        raise ValueError(f"not a canonical crossing-change u63: {action_u63}")
    return (action_u63 >> 4) & 0xFFFFFFFF


def load_groups(
    sidecar: Path, split: str, limit: int | None
) -> list[tuple[bytes, list[tuple]]]:
    connection = sqlite3.connect(f"file:{sidecar}?mode=ro", uri=True)
    rows = connection.execute(
        "SELECT o.state_sha256,o.state_encoding,o.action_u63,o.accepted,"
        "o.result_cc,o.result_semantic_moves "
        "FROM options o JOIN state_frontiers f USING(state_sha256) "
        "WHERE o.split=? AND f.accepted_actions>0 "
        "AND f.compared_actions>f.accepted_actions "
        "ORDER BY o.state_sha256,o.action_u63",
        (split,),
    )
    grouped: dict[bytes, list[tuple]] = defaultdict(list)
    for state_hash, encoding, action, accepted, cc, moves in rows:
        grouped[bytes(state_hash)].append(
            (bytes(encoding), int(action), bool(accepted), int(cc), int(moves))
        )
    connection.close()
    selected = sorted(grouped.items())
    return selected if limit is None else selected[:limit]


def compile_examples(scientist, groups, *, internal_cap: int, ratio: float):
    from pgx_mcts_bench.distill import _best_destination
    from rf_knots.evidence import SemanticAction

    game = scientist.game
    examples: dict[bytes, dict[str, object]] = {}
    stats = defaultdict(int)
    internal_histogram = defaultdict(int)
    for _state_hash, options in groups:
        word, strands, cyclic = decode_ukb0(options[0][0])
        if (
            cyclic
            or strands > game.config.max_strands
            or len(word) > game.config.max_len
        ):
            stats["capacity_excluded_states"] += 1
            continue
        initial = game.from_word(word, strands, math.log(ratio))
        paths = []
        for _encoding, action_u63, accepted, cc, moves in options:
            position = semantic_cc_position(action_u63)
            semantic = SemanticAction("CROSSING_CHANGE", position=position).to_flat(
                game.config._spec
            )
            length = int(np.asarray(initial.state.pgx._word).astype(bool).sum())
            destination = _best_destination(game, semantic, initial.state.head, length)
            if destination is None:
                stats["unroutable_options"] += 1
                continue
            route, _, native_action = destination
            if len(route) > internal_cap:
                stats["internal_cap_excluded_options"] += 1
                continue
            internal_histogram[len(route)] += 1
            paths.append(([*route, native_action], accepted, cc, moves))
        if not any(path[1] for path in paths):
            stats["no_accepted_route_states"] += 1
            continue
        for native_path, accepted, _cc, _moves in paths:
            transition = initial
            for action in native_path:
                if transition.terminated or not bool(transition.legal_actions[action]):
                    raise ValueError("compiled native route contains an illegal action")
                observation = np.asarray(transition.observation, dtype=np.float32)
                legal = np.asarray(transition.legal_actions, dtype=np.bool_)
                key = hashlib.sha256(observation.tobytes() + legal.tobytes()).digest()
                row = examples.setdefault(
                    key,
                    {
                        "observation": observation,
                        "legal": legal,
                        "accepted": set(),
                        "compared": set(),
                    },
                )
                row["compared"].add(action)
                if accepted:
                    row["accepted"].add(action)
                transition = game.step(transition.state, action)
        stats["compiled_semantic_states"] += 1

    usable = []
    for key in sorted(examples):
        row = examples[key]
        accepted = row["accepted"]
        compared = row["compared"]
        if accepted and compared - accepted:
            usable.append(row)
    stats["native_states_seen"] = len(examples)
    stats["native_training_rows"] = len(usable)
    stats["abstained_native_rows"] = len(examples) - len(usable)
    return usable, dict(stats), dict(sorted(internal_histogram.items()))


def arrays(rows, actions: int):
    observations = np.stack([row["observation"] for row in rows])
    accepted = np.zeros((len(rows), actions), dtype=np.bool_)
    compared = np.zeros((len(rows), actions), dtype=np.bool_)
    for index, row in enumerate(rows):
        accepted[index, list(row["accepted"])] = True
        compared[index, list(row["compared"])] = True
    return observations, accepted, compared


@torch.no_grad()
def evaluate(network, data, device: torch.device, batch_size: int) -> dict[str, float]:
    from pgx_mcts_bench.proof_guidance import conservative_set_policy_loss

    observations, accepted, compared = data
    total_loss = total_top = total_mass = 0.0
    count = len(observations)
    for start in range(0, count, batch_size):
        stop = min(start + batch_size, count)
        obs = torch.from_numpy(observations[start:stop]).permute(0, 3, 1, 2).to(device)
        acc = torch.from_numpy(accepted[start:stop]).to(device)
        cmp = torch.from_numpy(compared[start:stop]).to(device)
        logits, _ = network(obs)
        loss = conservative_set_policy_loss(logits, acc, cmp, reduction="sum")
        masked = logits.masked_fill(~cmp, -torch.inf)
        top = acc.gather(1, masked.argmax(dim=1, keepdim=True)).float().sum()
        accepted_mass = torch.logsumexp(logits.masked_fill(~acc, -torch.inf), dim=1)
        compared_mass = torch.logsumexp(logits.masked_fill(~cmp, -torch.inf), dim=1)
        total_loss += float(loss)
        total_top += float(top)
        total_mass += float(torch.exp(accepted_mass - compared_mass).sum())
    return {
        "rows": count,
        "loss": total_loss / max(count, 1),
        "restricted_top1_accepted": total_top / max(count, 1),
        "accepted_mass_within_compared": total_mass / max(count, 1),
    }


def train(network, train_data, *, device, epochs, batch_size, learning_rate, seed):
    from pgx_mcts_bench.proof_guidance import conservative_set_policy_loss

    for parameter in network.parameters():
        parameter.requires_grad_(False)
    adapter = network.attach_option_policy_adapter()
    for parameter in adapter.parameters():
        parameter.requires_grad_(True)
    optimizer = torch.optim.AdamW(
        adapter.parameters(), lr=learning_rate, weight_decay=1e-4
    )
    observations, accepted, compared = train_data
    generator = np.random.default_rng(seed)
    history = []
    network.train()
    for epoch in range(epochs):
        order = generator.permutation(len(observations))
        loss_sum = 0.0
        for start in range(0, len(order), batch_size):
            indices = order[start : start + batch_size]
            obs = torch.from_numpy(observations[indices]).permute(0, 3, 1, 2).to(device)
            acc = torch.from_numpy(accepted[indices]).to(device)
            cmp = torch.from_numpy(compared[indices]).to(device)
            optimizer.zero_grad(set_to_none=True)
            with torch.no_grad():
                enabled = network.option_adapter_enabled
                network.option_adapter_enabled = False
                base_logits, _ = network(obs)
                network.option_adapter_enabled = enabled
            residual, _ = network.option_policy_components(obs)
            loss = conservative_set_policy_loss(
                base_logits.detach() + residual, acc, cmp
            )
            loss.backward()
            torch.nn.utils.clip_grad_norm_(adapter.parameters(), 1.0)
            optimizer.step()
            loss_sum += float(loss.detach()) * len(indices)
        history.append({"epoch": epoch + 1, "train_loss": loss_sum / len(order)})
    network.eval()
    return adapter, history


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sidecar", type=Path, required=True)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--pgx-root", type=Path, required=True)
    parser.add_argument("--rf-root", type=Path, required=True)
    parser.add_argument("--internal-action-cap", type=int, default=5)
    parser.add_argument("--train-limit", type=int, default=20000)
    parser.add_argument("--validation-limit", type=int, default=4000)
    parser.add_argument("--test-limit", type=int, default=4000)
    parser.add_argument("--epochs", type=int, default=4)
    parser.add_argument("--batch-size", type=int, default=128)
    parser.add_argument("--learning-rate", type=float, default=1e-3)
    parser.add_argument("--seed", type=int, default=20260828)
    args = parser.parse_args()
    if args.internal_action_cap < 0 or args.epochs <= 0:
        raise ValueError("invalid route or training budget")

    import sys

    sys.path.insert(0, str((args.pgx_root / "src").resolve()))
    sys.path.insert(0, str((args.rf_root / "src").resolve()))
    from pgx_mcts_bench.adaptive_scientists import load_scientist

    args.output_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = args.model_dir / "manifest.json"
    model_manifest = json.loads(manifest_path.read_text())
    checkpoint = args.model_dir / model_manifest["checkpoint"]
    if file_sha256(checkpoint) != model_manifest["checkpoint_sha256"]:
        raise ValueError("frozen checkpoint hash mismatch")
    if int(model_manifest["objective_ratio"]) != 1000:
        raise ValueError("model is not the pinned L1000 policy")
    sidecar_sha256 = file_sha256(args.sidecar)
    started = time.monotonic()
    torch.manual_seed(args.seed)
    torch.set_num_threads(min(8, os.cpu_count() or 1))
    device = torch.device("cpu")
    scientist = load_scientist(
        model_manifest["scientist"],
        checkpoint,
        seed=args.seed,
        device=str(device),
        require_factorized=True,
        objective_budget_channel=True,
    )
    scientist.network.eval()
    scientist.network.attach_option_policy_adapter()

    split_rows = {}
    compilation = {}
    histograms = {}
    limits = {
        "train": args.train_limit,
        "validation": args.validation_limit,
        "test": args.test_limit,
    }
    for split in ("train", "validation", "test"):
        groups = load_groups(args.sidecar, split, limits[split])
        rows, stats, histogram = compile_examples(
            scientist, groups, internal_cap=args.internal_action_cap, ratio=1000.0
        )
        split_rows[split] = arrays(rows, scientist.game.num_actions)
        compilation[split] = {"input_groups": len(groups), **stats}
        histograms[split] = histogram
    if not len(split_rows["train"][0]):
        raise ValueError("no trainable native rows after compilation")

    with torch.no_grad():
        probe = torch.from_numpy(split_rows["train"][0][:32]).permute(0, 3, 1, 2)
        scientist.network.option_adapter_enabled = False
        base_probe, _ = scientist.network(probe)
        scientist.network.option_adapter_enabled = True
        attached_probe, _ = scientist.network(probe)
        zero_initialization_max_logit_delta = float(
            (base_probe - attached_probe).abs().max()
        )
    if zero_initialization_max_logit_delta != 0.0:
        raise ValueError("new option adapter does not preserve frozen Q254 logits exactly")

    zero_metrics = {
        split: evaluate(scientist.network, data, device, args.batch_size)
        for split, data in split_rows.items()
    }
    adapter, history = train(
        scientist.network,
        split_rows["train"],
        device=device,
        epochs=args.epochs,
        batch_size=args.batch_size,
        learning_rate=args.learning_rate,
        seed=args.seed,
    )
    trained_metrics = {
        split: evaluate(scientist.network, data, device, args.batch_size)
        for split, data in split_rows.items()
    }
    adapter_path = args.output_dir / "adapter.pt"
    temporary = adapter_path.with_name(f".{adapter_path.name}.{os.getpid()}.part")
    torch.save({"adapter": adapter.state_dict()}, temporary)
    os.replace(temporary, adapter_path)
    report = {
        "schema": SCHEMA,
        "status": "trained_not_published",
        "proof_policy_contract": {
            "model_id": model_manifest["model_id"],
            "checkpoint_sha256": model_manifest["checkpoint_sha256"],
            "base_parameters_frozen": True,
            "zero_initialization_max_logit_delta": zero_initialization_max_logit_delta,
            "objective_ratio": 1000,
            "internal_nonsemantic_action_cap": args.internal_action_cap,
        },
        "dataset": {
            "path": str(args.sidecar.resolve()),
            "sha256": sidecar_sha256,
            "compilation": compilation,
            "internal_route_length_histograms": histograms,
            "unknown_actions_receive_graph_gradient": False,
        },
        "training": {
            "seed": args.seed,
            "epochs": args.epochs,
            "batch_size": args.batch_size,
            "learning_rate": args.learning_rate,
            "adapter_parameters": sum(
                parameter.numel() for parameter in adapter.parameters()
            ),
            "history": history,
        },
        "metrics_before_zero_initialized_adapter": zero_metrics,
        "metrics_after": trained_metrics,
        "adapter": {
            "path": str(adapter_path.resolve()),
            "sha256": file_sha256(adapter_path),
        },
        "elapsed_seconds": time.monotonic() - started,
    }
    atomic_json(args.output_dir / "manifest.json", report)
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
