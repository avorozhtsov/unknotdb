# RF best-first plus descending-fallback population, 2026-08-26

## Facts

- RF source commit: `f183188e2e9d727d22fd53e151ad80ffea32a886` (read-only).
- Versioned corpus: 2,989 unique, parseable, single-component braid
  representations. The extractor recorded 540 excluded source references: 426
  without a braid outside the local crossing table, 107 whose stored builder
  exceeded its strand cap, 5 beyond the supported local strand cap, and 2
  multi-component closures. These exclusions are not representations that can
  be inserted.
- Before this population campaign, the last full RF audit reported 208 covered
  representations.
- The replay-verified RF best-solution pool contains best traces for 108 distinct
  corpus representations. Its source SHA-256 is
  `1a08b891078438229391b24ee62a6d18ea24083ebdea583b57219dda05be38c7`.
- Best traces were processed first. 105/108 compiled and replayed under the
  current stopping-point contract; 54 strictly improved U, by 124 CC in total
  (median improvement 2, maximum 5). The three rejected best traces required an
  inverse coordinate-prefix transport that the exact compiler cannot prove;
  no heuristic edge was stored.
- The remaining corpus was processed in durable fixed ranges by the exact
  descending-diagram fallback. It added 2,765 connected input routes. Each CC
  is a separate immutable edge, and every raw successor is mandatorily
  preprocessed and normalized before becoming a graph vertex.
- Final exact coverage is 2,983/2,989 (99.799%). Six inputs remain unpublished:
  two length-53, 10-strand DKT braids exceed Q254's word capacity during initial
  Markov stabilization; one input reaches a detected controller cycle before a
  stopping point; three descending chains reach a controller cycle after a CC.
- On the 2,983 covered RF inputs, U p50/p95/max is 8/16/22; 978 have U greater
  than 10. These are upper bounds from the deliberately straightforward
  fallback, not claims of optimal unknotting number.
- Final snapshot: 94,642 nodes, 146,940 edges, 20,529 deduplicated programs,
  24,420,352 bytes. Relative to the parent snapshot this is +23,454 nodes,
  +27,414 edges, +10,292 programs, and +6,119,424 bytes.

## Unpublished inputs

| Representation | Source | Reason |
|---|---|---|
| `braid:8cb45c...8969b` | DKT benchmark | Q254 capacity |
| `braid:b0c9ac...a2d0c` | DKT benchmark | Q254 capacity |
| `braid:c149f6...ad75a` | bundled knot table | initial controller cycle |
| `braid:2d08d5...16134` | bundled knot table | post-CC controller cycle |
| `braid:de28ae...e76112` | bundled knot table | post-CC controller cycle |
| `braid:7917f6...a96e4` | bundled knot table | post-CC controller cycle |

None of these six has a compatible replay-verified best trace in the inspected
RF best-solution pool. Supporting them requires a separately versioned policy
adapter/capacity migration; silently bypassing the pinned preprocessing contract
would produce incompatible graph vertices.

## Validation

- Snapshot SHA-256:
  `0230f7529a188d1c8f2ff246b1661ff7b99cf783cbbea03342b9fd64ad3b3444`.
- SQLite integrity: `ok`; foreign-key check: empty.
- B4 regression: 286,334/286,334 preprocessed seeds retained; all 146,940
  edges independently replayed; U p50/p95/max = 4/9/22.
- Rust: 61 unit tests and the frozen RF reference-vector test passed.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and Ruff on
  the RF extractors passed.
- No policy training or checkpoint change, RF Knots/pgx-mcts-bench write,
  commit, push, or deployment occurred.

## Artifacts

- Final snapshot: `outputs/unknotdb-rf-complete-audited-v0.sqlite`
- Complete coverage audit: `outputs/unknotdb-rf-complete-audit-v0.tsv`
- B4/replay gate: `outputs/unknotdb-rf-complete-b4-regression-v0.tsv`
- Corpus: `outputs/rf-knots-representation-corpus-20260826-v1.{json,tsv}`
- Compact verified-best input: `outputs/rf-best-witnesses-20260815-v0.tsv`
- Best-first audit: `outputs/unknotdb-rf-best-first-v0.tsv`
- Fallback range audits: `outputs/unknotdb-rf-descending-*-v0.tsv`

## Proposal

Keep this Q254 snapshot immutable. To cover the six remaining inputs, define a
new policy-adapter version that (a) handles the two over-capacity inputs and (b)
uses deterministic acyclic alternative-action selection when top-1 zero-cost
policy moves form a cycle, then reattest the protected RF and B4 corpora in a
separate snapshot. Quality work should next prioritize the 978 RF inputs with
U greater than 10 using verified RF/search traces rather than treating the
descending bounds as optimal.
