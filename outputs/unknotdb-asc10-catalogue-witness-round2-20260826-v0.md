# ASC10 catalogue-witness round 2

## Scope

- Input snapshot: `outputs/unknotdb-8_9-u1-v0.sqlite`
- Final snapshot: `outputs/unknotdb-12a_1047-u2-v0.sqlite`
- Final SHA-256: `767127197c81117d2ae2e95ffe9b2785b4436d798493f14cc11b0686ff6a3490`
- Queue order: increasing `(ASC10, node_id, knot_id)` from the versioned KnotInfo sidecar.
- Local optimizer budget per attempted node: depth 6, 200,000 states,
  200,000 simulations, at most 6 strands and word length 32.
- Frozen policy provenance was unchanged:
  `q-grown-raster-axial-12:Q254:36d1122f5494d502e556994083a1a69adf1643d5be95cd9f80ffc13b68e68d63`.

## Verified witnesses accepted

| Named knot | Old U upper | New U upper | Source | Inserted edges | Inserted nodes |
|---|---:|---:|---|---:|---:|
| `10_48` | 3 | 2 | RF best trace `a4363cf91e526858fc52c8ecc4c4881acca9a46e05f1233d70717fd75f403f07` | 3 | 0 |
| `12a_1047` | 3 | 2 | RF best trace `3526759332e8d590c2da1d6f29547ca4b36f81754450e02d6595b69ade8d3cb5` | 3 | 1 |

Both traces were compiled to immutable programs and independently replayed.
Each proof edge contains exactly zero or one crossing change. Bellman relaxation
also resolved four catalogue discrepancies indirectly: `10_106` (3 to 2),
`11n_60` (3 to 2), `12a_878` (4 to 3), and `13n_1203` (4 to 3).

The accepted RF traces originally began with a stabilization after a canonical
coordinate prefix. The semantic compiler now transports that prefix through the
stabilization by selecting the unique exact word gap and emits `StabilizeAt`.
The new behavior has a dedicated round-trip test.

## Truthful misses

The fixed local budget found no strict improvement for 25 queue entries:
`10_141`, `12a_864`, `12a_1128`, `12n_709`, `12n_748`, `12a_1176`,
`8_5`, `8_2`, `10_91`, `10_106`, `10_9`, `10_123`, `10_85`, `10_118`,
`10_104`, `10_99`, `6_3`, `10_112`, `12a_1233`, `12a_920`, `12n_751`,
`12n_822`, `12n_821`, `12a_1249`, and `12a_1215`.
No snapshot was published for any miss. `10_106` was subsequently improved as
a collateral consequence of the accepted `12a_1047` trace.

## Final measured state

- Nodes: 100,623 (growth: 1)
- Edges: 153,599 (growth: 6)
- Deduplicated programs: 17,235 (growth: 6)
- Snapshot bytes: 27,557,888
- U distribution: p50 4, p95 11, max 31
- Catalogue comparisons: 3,438
- Remaining shorter-witness queue: 3,371 (down by 6)
- Refreshed catalogue artifacts:
  `outputs/unknotdb-catalogue-u-v3.sqlite`,
  `outputs/unknotdb-catalogue-u-asc10-queue-v3.tsv`, and
  `outputs/unknotdb-catalogue-u-v3.md`.

The catalogue queue builder was also corrected to join an identity sidecar to a
new snapshot through the durable canonical stopping key. Snapshot-local node IDs
are now only a legacy fallback, so inserting a vertex cannot cause false key drift.

## Validation

- SQLite integrity: `ok`.
- Full immutable-edge replay: 153,599/153,599.
- Frozen B4 preprocessing coverage: 286,334/286,334.
- Rust tests: 64/64 passed (63 unit and 1 RF reference-vector test).
- `cargo fmt --check`: passed.
- strict Clippy with `-D warnings`: passed.
- Ruff for the catalogue queue builder: passed.
- B4 gate: `outputs/unknotdb-12a_1047-u2-b4-regression-v0.tsv`.

No policy was trained or modified. RF Knots and pgx-mcts-bench were read-only.
No commit, push, or deployment was performed.
