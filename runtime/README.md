# unknotdb runtime

Generated immutable SQLite snapshots and an optional in-memory hash index. This
crate is outside the verifier's trusted, dependency-free core.

```bash
cargo test --manifest-path runtime/Cargo.toml
cargo build --release --manifest-path runtime/Cargo.toml

BIN=runtime/target/release/unknotdb-runtime
$BIN normalize-braid 3 2,1,-2,1
$BIN lookup-braid /tmp/unknotdb-bootstrap.sqlite 2 -1,-1,-1
$BIN reduce-braid 3 2,1,-1
$BIN preprocess-braid 2 -1,-1,-1 \
  --max-policy-plies 128 --max-semantic-moves 32 \
  --oracle /path/to/pgx-mcts-bench/.venv/bin/python \
  tools/q_policy_oracle.py \
  --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
$BIN trace-edge-braid 2 -1,-1,-1 \
  --max-policy-plies 128 --max-semantic-moves 32 \
  --oracle /path/to/pgx-mcts-bench/.venv/bin/python \
  tools/q_policy_oracle.py \
  --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
$BIN bootstrap-snapshot /tmp/unknotdb-bootstrap.sqlite \
  --max-policy-plies 128 --max-semantic-moves 32 \
  --oracle /path/to/pgx-mcts-bench/.venv/bin/python \
  tools/q_policy_oracle.py \
  --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
$BIN populate-unknot-frontier /tmp/unknotdb-frontier.sqlite \
  --manifest /tmp/unknotdb-frontier.tsv \
  --scramble-depth 3 --max-scramble-states 512 \
  --max-scramble-candidates 8 --min-scramble-cc 1 --max-scramble-cc 1 \
  --oracle /path/to/pgx-mcts-bench/.venv/bin/python \
  tools/q_policy_oracle.py \
  --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
$BIN resume-frontier /tmp/unknotdb-frontier.sqlite /tmp/unknotdb-next.sqlite \
  --manifest /tmp/unknotdb-next.tsv --generations 1 \
  --scramble-depth 3 --max-scramble-states 512 \
  --max-scramble-candidates 8 --min-scramble-cc 1 --max-scramble-cc 1 \
  --oracle /path/to/pgx-mcts-bench/.venv/bin/python \
  tools/q_policy_oracle.py \
  --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
$BIN census-natural-braids /tmp/unknotdb-next.sqlite \
  --report /tmp/b3-census.md --manifest /tmp/b3-census.tsv \
  --max-strands 3 --max-word-length 8 --scramble-depth 3 \
  --oracle /path/to/pgx-mcts-bench/.venv/bin/python \
  tools/q_policy_oracle.py \
  --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
$BIN complete-natural-braids /tmp/unknotdb-next.sqlite /tmp/unknotdb-b3.sqlite \
  --manifest /tmp/unknotdb-b3.tsv --max-strands 3 --max-word-length 8 \
  --max-policy-plies 16 --max-semantic-moves 4 \
  --oracle /path/to/pgx-mcts-bench/.venv/bin/python \
  tools/q_policy_oracle.py \
  --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
$BIN build-synthetic /tmp/unknotdb.sqlite --nodes 100000
$BIN inspect /tmp/unknotdb.sqlite
$BIN bench /tmp/unknotdb.sqlite --queries 200000
$BIN bench-population /tmp/unknotdb.sqlite \
  --duplicate-rounds 10 --refresh-rounds 10 --rebuild-rounds 1
```

Synthetic edges exercise storage invariants only and are explicitly marked
non-proofs. `preprocess-braid` is inference-only: the Python process chooses
top-1 policy actions from the hash-pinned frozen checkpoint, while Rust enforces
bounds and independently replays every semantic action. `trace-edge-braid`
then emits one v1 proof macro with exactly one CC and explicit, validated
intermediate origin changes. `bootstrap-snapshot` passes that macro through the
real reverse-DP/Bellman relaxation engine and publishes an independently
validated two-node production snapshot. `populate-unknot-frontier` enumerates a
bounded deterministic scramble layer, runs the initial-only reducer followed by
normalization-only policy transitions, compiles a state-aware inverse witness,
and records every attempt in a durable manifest.
`resume-frontier` first validates every parent row and proof hash, refuses a
different policy model ID, expands one or more route-rank generations, and
publishes a new immutable snapshot plus a generation/seed manifest. Snapshot
publication also recomputes `next_acs10` independently from `next_unknot` over
all retained proof edges. `census-natural-braids` exhaustively measures
normalized natural inputs through three strands and their full bounded scramble
trees. It groups inputs by deterministic reducer output to avoid duplicate
policy calls while retaining and re-verifying a separate reducer witness for
every input. `complete-natural-braids` uses the same proof-preserving grouping
and adds missing stopping points only through independently replayed one-CC
policy macros; `complete-natural-b2` remains a compatibility alias. See
[`../docs/graph-runtime-v0.md`](../docs/graph-runtime-v0.md) and
the normative
[`../schema/graph-representation-v0.md`](../schema/graph-representation-v0.md)
and [`../schema/preprocessing-v0.md`](../schema/preprocessing-v0.md).
