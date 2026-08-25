"""Serve deterministic top-1 policy decisions from a frozen Q checkpoint.

This process performs inference only. It starts a fresh canonical controller for
every normalized representation, applies only internal head/memory actions, and
returns the first semantic preference to the Rust runtime. Rust independently
checks and applies any returned zero-CC semantic action.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
import sys
import time
from collections import OrderedDict
from pathlib import Path
from typing import Any

PROTOCOL = "UNKNOTDB_POLICY_V0"
CONTROLLER_INITIAL_STATE = "canonical-clean-v0"
REPRESENTATION_MAGIC = b"UKB0"
OBJECTIVE_RATIO = 1000


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def decode_representation(encoded_hex: str) -> tuple[list[int], int]:
    try:
        encoded = bytes.fromhex(encoded_hex)
    except ValueError as error:
        raise ValueError("representation is not hexadecimal") from error
    if len(encoded) < 12 or encoded[:4] != REPRESENTATION_MAGIC:
        raise ValueError("bad UKB0 representation header")
    version, flags, strands, length = struct.unpack("<BBHI", encoded[4:12])
    if version != 0 or flags & ~1:
        raise ValueError("unsupported UKB0 version or flags")
    if flags & 1:
        raise ValueError("q-grown-raster-axial-12 does not support cyclic band generators")
    if len(encoded) != 12 + 2 * length:
        raise ValueError("UKB0 representation length mismatch")
    word = list(struct.unpack(f"<{length}h", encoded[12:]))
    if strands < 1 or any(letter == 0 or abs(letter) >= strands for letter in word):
        raise ValueError("invalid ordinary braid representation")
    if word:
        rotations = lambda values: min(
            tuple(values[offset:] + values[:offset])
            for offset in range(len(values))
        )
        original = rotations(word)
        reflected_word = [-letter for letter in word]
        reflected = rotations(reflected_word)
        writhe = sum(word)
        expected = original if writhe > 0 else reflected if writhe < 0 else min(
            original, reflected
        )
        if tuple(word) != expected:
            raise ValueError("policy input is not mirror-orbit normalized")
    return word, strands


def encode_semantic_action(kind: int, position: int, generator: int, sign: int) -> int:
    negative = kind == 3 and sign < 0
    return kind | (position << 4) | (generator << 36 if kind == 3 else 0) | (
        int(negative) << 52
    )


def controller_fingerprint(numpy: Any, transition: Any) -> tuple[Any, ...]:
    state = transition.state
    pgx_state = state.pgx
    return (
        tuple(int(value) for value in numpy.asarray(pgx_state._word)),
        int(numpy.asarray(pgx_state._n)),
        int(state.head),
        numpy.asarray(state.registers).tobytes(),
        numpy.asarray(state.colours).tobytes(),
        int(state.colour),
        numpy.asarray(state.tape).tobytes(),
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pgx-root", type=Path, required=True)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--seed", type=int, default=20260824)
    parser.add_argument(
        "--profile-timings-every",
        type=int,
        default=0,
        help="emit cumulative phase timings to stderr every N decisions",
    )
    parser.add_argument(
        "--decision-cache-size",
        type=int,
        default=65536,
        help="bounded exact clean-controller decision cache; zero disables it",
    )
    parser.add_argument(
        "--network-batch-size",
        type=int,
        default=8,
        help="maximum independent observations per Torch forward",
    )
    args = parser.parse_args()
    if args.profile_timings_every < 0:
        raise ValueError("--profile-timings-every must be non-negative")
    if args.decision_cache_size < 0:
        raise ValueError("--decision-cache-size must be non-negative")
    if args.network_batch_size <= 0:
        raise ValueError("--network-batch-size must be positive")

    manifest_path = args.model_dir.resolve() / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("schema") != "unknotdb-frozen-policy-manifest-v0":
        raise ValueError("unsupported frozen-policy manifest")
    if int(manifest.get("objective_ratio", 0)) != OBJECTIVE_RATIO:
        raise ValueError("frozen policy is not pinned to L1000")
    if manifest.get("controller_initial_state") != CONTROLLER_INITIAL_STATE:
        raise ValueError("frozen policy has a different controller initial-state rule")
    checkpoint = args.model_dir.resolve() / str(manifest["checkpoint"])
    checkpoint_sha256 = sha256_file(checkpoint)
    if checkpoint_sha256 != manifest["checkpoint_sha256"]:
        raise ValueError("frozen policy checkpoint SHA-256 mismatch")
    expected_model_id = (
        f"{manifest['lineage']}:{manifest['q_generation']}:{checkpoint_sha256}"
    )
    if manifest.get("model_id") != expected_model_id:
        raise ValueError("frozen policy model ID does not match checkpoint SHA-256")

    sys.path.insert(0, str(args.pgx_root.resolve() / "src"))
    import numpy  # type: ignore[import-not-found]
    import torch  # type: ignore[import-not-found]
    import jax  # type: ignore[import-not-found]
    import jax.numpy as jnp  # type: ignore[import-not-found]
    from pgx_mcts_bench.adaptive_scientists import (
        load_scientist,
    )
    from rf_knots.actions import CROSSING_CHANGE, PASS

    torch.set_num_threads(1)
    scientist = load_scientist(
        str(manifest["scientist"]),
        checkpoint,
        seed=args.seed,
        device=args.device,
        simulations=0,
        require_factorized=True,
        objective_budget_channel=True,
    )
    scientist.network.eval()
    device = torch.device(args.device)
    game = scientist.game
    env = game.env
    log_objective_ratio = math.log(OBJECTIVE_RATIO)
    template_state = env.init_from_word([], 1, log_ratio=log_objective_ratio)

    def initialize_state(padded_word: Any, strands: Any) -> Any:
        phase = jnp.int32(1)
        state = template_state.replace(
            current_player=jnp.int32(1),
            rewards=jnp.zeros_like(template_state.rewards),
            terminated=jnp.bool_(False),
            truncated=jnp.bool_(False),
            _word=padded_word,
            _n=strands.astype(jnp.int32),
            _phase=phase,
            _budget=jnp.int32(env.config.simplify_budget),
            _scrambler=jnp.int32(0),
            _crossing_changes=jnp.int32(0),
            _log_ratio=jnp.float32(log_objective_ratio),
        )
        state = state.replace(legal_action_mask=env._mask(padded_word, strands, phase))
        return state.replace(observation=env.observe(state))

    initialize_states = jax.jit(jax.vmap(initialize_state))
    model_id = str(manifest["model_id"])
    print(
        f"{PROTOCOL} {model_id} {OBJECTIVE_RATIO} {CONTROLLER_INITIAL_STATE}",
        flush=True,
    )
    profile = {
        "requests": 0,
        "cache_hits": 0,
        "network_calls": 0,
        "controller_steps": 0,
        "decode_ns": 0,
        "environment_init_ns": 0,
        "environment_state_ns": 0,
        "serial_view_ns": 0,
        "tensor_ns": 0,
        "network_ns": 0,
        "action_decode_ns": 0,
        "controller_step_ns": 0,
        "total_ns": 0,
    }
    decision_cache: OrderedDict[tuple[str, tuple[int, ...]], tuple[str, int]] = OrderedDict()

    def decide_many(requests: list[tuple[str, int, tuple[int, ...]]]) -> list[str]:
        batch_started = time.perf_counter_ns()
        responses: list[str | None] = [None] * len(requests)
        states: list[dict[str, Any] | None] = [None] * len(requests)
        pending: list[tuple[int, str, int, tuple[int, ...], list[int], int]] = []
        for index, (encoded, remaining, excluded) in enumerate(requests):
            if remaining < 0:
                raise ValueError("remaining policy plies must be non-negative")
            cache_key = (encoded, excluded)
            cached = decision_cache.get(cache_key)
            if cached is not None and remaining >= cached[1]:
                responses[index] = cached[0]
                decision_cache.move_to_end(cache_key)
                profile["cache_hits"] += 1
                continue
            phase_started = time.perf_counter_ns()
            word, strands = decode_representation(encoded)
            profile["decode_ns"] += time.perf_counter_ns() - phase_started
            prepared_word, prepared_strands = game._initial_representation(word, strands)
            pending.append(
                (index, encoded, remaining, excluded, prepared_word, prepared_strands)
            )

        pgx_states: list[Any] = []
        if pending:
            init_started = time.perf_counter_ns()
            if len(pending) == 1:
                _, _, _, _, prepared_word, prepared_strands = pending[0]
                pgx_states = [
                    env.init_from_word(
                        prepared_word,
                        prepared_strands,
                        log_ratio=log_objective_ratio,
                    )
                ]
            else:
                padded = numpy.zeros(
                    (len(pending), env.config.max_len), dtype=numpy.int32
                )
                strands_batch = numpy.empty(len(pending), dtype=numpy.int32)
                for row, (_, _, _, _, prepared_word, prepared_strands) in enumerate(pending):
                    padded[row, : len(prepared_word)] = prepared_word
                    strands_batch[row] = prepared_strands
                batched_state = jax.device_get(
                    initialize_states(jnp.asarray(padded), jnp.asarray(strands_batch))
                )
                pgx_states = [
                    jax.tree_util.tree_map(lambda values, row=row: values[row], batched_state)
                    for row in range(len(pending))
                ]
            profile["environment_state_ns"] += time.perf_counter_ns() - init_started

        for (index, encoded, remaining, excluded, _, _), pgx_state in zip(
            pending, pgx_states
        ):
            phase_started = time.perf_counter_ns()
            transition = game._view(
                pgx_state,
                0,
                game._no_registers(),
                game._no_colours(),
                0,
                game._no_tape(),
                reward=0.0,
            )
            profile["serial_view_ns"] += time.perf_counter_ns() - phase_started
            states[index] = {
                "encoded": encoded,
                "remaining": remaining,
                "excluded": frozenset(excluded),
                "transition": transition,
                "seen": {controller_fingerprint(numpy, transition)},
                "controller_plies": 0,
            }
        profile["environment_init_ns"] = (
            profile["environment_state_ns"] + profile["serial_view_ns"]
        )

        while True:
            active: list[int] = []
            for index, state in enumerate(states):
                if state is None or responses[index] is not None:
                    continue
                transition = state["transition"]
                controller_plies = state["controller_plies"]
                if transition.terminated:
                    responses[index] = f"STOP_TERMINATED {controller_plies}"
                elif controller_plies >= state["remaining"]:
                    responses[index] = f"STOP_PLY_LIMIT {controller_plies}"
                else:
                    active.append(index)
            if not active:
                break

            actions: list[int] = []
            with torch.inference_mode():
                for offset in range(0, len(active), args.network_batch_size):
                    chunk = active[offset : offset + args.network_batch_size]
                    phase_started = time.perf_counter_ns()
                    observations = numpy.stack(
                        [states[index]["transition"].observation for index in chunk]
                    )
                    tensor = (
                        torch.from_numpy(observations)
                        .permute(0, 3, 1, 2)
                        .float()
                        .to(device)
                    )
                    profile["tensor_ns"] += time.perf_counter_ns() - phase_started
                    phase_started = time.perf_counter_ns()
                    logits, _ = scientist.network(tensor)
                    profile["network_ns"] += time.perf_counter_ns() - phase_started
                    profile["network_calls"] += 1
                    phase_started = time.perf_counter_ns()
                    legal = torch.from_numpy(
                        numpy.stack(
                            [states[index]["transition"].legal_actions for index in chunk]
                        )
                    ).to(device=device)
                    floor = torch.finfo(logits.dtype).min
                    masked = logits.masked_fill(~legal, floor)
                    for row, index in enumerate(chunk):
                        excluded = states[index]["excluded"]
                        if not excluded:
                            actions.append(int(masked[row].argmax().item()))
                            continue
                        ranked = masked[row].argsort(descending=True).cpu().tolist()
                        transition = states[index]["transition"]
                        pgx_state = transition.state.pgx
                        length = int(
                            numpy.count_nonzero(numpy.asarray(pgx_state._word))
                        )
                        selected = None
                        for candidate in ranked:
                            if not bool(transition.legal_actions[candidate]):
                                continue
                            underlying = game.underlying_action(
                                int(candidate), int(transition.state.head), length
                            )
                            if underlying is None:
                                selected = int(candidate)
                                break
                            kind, position, generator, sign = game.spec.decode(underlying)
                            semantic = encode_semantic_action(
                                int(kind), int(position), int(generator), int(sign)
                            )
                            if semantic not in excluded:
                                selected = int(candidate)
                                break
                        if selected is None:
                            raise RuntimeError("all legal policy actions were excluded")
                        actions.append(selected)
                    profile["action_decode_ns"] += (
                        time.perf_counter_ns() - phase_started
                    )

            for index, action in zip(active, actions):
                state = states[index]
                transition = state["transition"]
                pgx_state = transition.state.pgx
                length = int(numpy.count_nonzero(numpy.asarray(pgx_state._word)))
                underlying = scientist.game.underlying_action(
                    int(action), int(transition.state.head), length
                )
                if underlying is not None:
                    kind, position, generator, sign = scientist.game.spec.decode(underlying)
                    semantic = encode_semantic_action(
                        int(kind), int(position), int(generator), int(sign)
                    )
                    controller_plies = state["controller_plies"]
                    if kind == CROSSING_CHANGE:
                        responses[index] = f"STOP_CC {semantic} {controller_plies}"
                    elif kind == PASS:
                        responses[index] = f"STOP_PASS {controller_plies}"
                    else:
                        responses[index] = f"APPLY {semantic} {controller_plies}"
                    continue

                phase_started = time.perf_counter_ns()
                transition = scientist.game.step(transition.state, int(action))
                profile["controller_step_ns"] += time.perf_counter_ns() - phase_started
                profile["controller_steps"] += 1
                state["controller_plies"] += 1
                state["transition"] = transition
                fingerprint = controller_fingerprint(numpy, transition)
                if fingerprint in state["seen"]:
                    responses[index] = (
                        f"STOP_CONTROLLER_CYCLE {state['controller_plies']}"
                    )
                else:
                    state["seen"].add(fingerprint)

        final = [response for response in responses if response is not None]
        if len(final) != len(requests):
            raise RuntimeError("internal batch decision did not finish every request")
        for (encoded, _, excluded), response in zip(requests, final):
            if args.decision_cache_size and not response.startswith("STOP_PLY_LIMIT "):
                controller_plies = int(response.rsplit(" ", 1)[1])
                cache_key = (encoded, excluded)
                decision_cache[cache_key] = (response, controller_plies + 1)
                decision_cache.move_to_end(cache_key)
                if len(decision_cache) > args.decision_cache_size:
                    decision_cache.popitem(last=False)
        profile["requests"] += len(requests)
        profile["total_ns"] += time.perf_counter_ns() - batch_started
        return final

    input_lines = iter(sys.stdin)
    for raw_line in input_lines:
        line = raw_line.strip()
        if line == "QUIT":
            return
        try:
            fields = line.split()
            if fields[:1] == ["DECIDE"] and len(fields) in {3, 4}:
                excluded_text = fields[3] if len(fields) == 4 else "-"
                excluded = () if excluded_text == "-" else tuple(
                    int(value) for value in excluded_text.split(",")
                )
                responses = decide_many([(fields[1], int(fields[2]), excluded)])
                print(responses[0], flush=True)
            elif fields[:1] == ["DECIDE_BATCH"] and len(fields) == 2:
                count = int(fields[1])
                if count <= 0:
                    raise ValueError("DECIDE_BATCH count must be positive")
                requests = []
                for _ in range(count):
                    request_fields = next(input_lines).strip().split()
                    if len(request_fields) not in {2, 3}:
                        raise ValueError("bad DECIDE_BATCH request row")
                    encoded, remaining_text = request_fields[:2]
                    excluded_text = request_fields[2] if len(request_fields) == 3 else "-"
                    excluded = () if excluded_text == "-" else tuple(
                        int(value) for value in excluded_text.split(",")
                    )
                    requests.append((encoded, int(remaining_text), excluded))
                responses = decide_many(requests)
                print(f"BATCH {len(responses)}")
                for response in responses:
                    print(response)
                sys.stdout.flush()
            else:
                raise ValueError("expected DECIDE or DECIDE_BATCH")
            if (
                args.profile_timings_every
                and profile["requests"] >= args.profile_timings_every
                and profile["requests"] % args.profile_timings_every < len(responses)
            ):
                payload = {"schema": "unknotdb-policy-profile-v0", **profile}
                print(json.dumps(payload, sort_keys=True), file=sys.stderr, flush=True)
        except Exception as error:  # noqa: BLE001 - line protocol error boundary
            message = " ".join(str(error).split())
            print(f"ERROR {type(error).__name__}: {message}", flush=True)


if __name__ == "__main__":
    main()
