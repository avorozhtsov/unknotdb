# Resolution of every U upper bound above 10

## Scope

Input snapshot: `unknotdb-descending-high-u-round3-v0.sqlite`, SHA-256
`cda2d5f4fb26619dcaac4084cfd229a274819c27b463bbf4a21dd584f9cf1f01`.
The audit found exactly 28,860 nodes with `U_upper > 10`, spanning 11 through
1799. The complete set was frozen by deterministic descending U, strands, word
length and canonical-key ordering.

## Execution

All 28,860 nodes were processed under the pinned Q254/L1000 preprocessing
contract. Each raw post-CC successor passed through mandatory preprocessing and
deterministic normalization before its stopping point was deduplicated or
inserted. Each accepted edge contains an independently replayable program and
at most one CC.

The population path was changed to stage all individually compiled and replayed
certificates transactionally, followed by one complete 0-1 BFS and independent
ACS10 refresh for the batch. This removes redundant full-graph recomputation
after every certificate without weakening validation or publication atomicity.

Results: 28,860 selected, 28,860 certified, zero excluded, zero failed, 28,860
direct improvements and 3,309 collateral improvements. The batch added 2,116
normalized stopping-point nodes, 47,747 immutable edges and 7,759 deduplicated
programs. The selected aggregate U decrease was 15,511,135.

## Final graph

Final snapshot: `unknotdb-descending-all-gt10-v0.sqlite`, SHA-256
`2f9c13ff7745d7b31cc333f44e5eaa782d1b9d1ce4e2efcc9d18c45a5abfce24`.

- 71,159 nodes, 118,178 edges, 10,048 programs.
- 18,214,912 bytes, up from 13,324,288 bytes.
- U[p50/p95/max] = 4/7/10.
- Nodes with `U_upper > 10`: 28,860 before, zero after.
- Final U counts: 0:113, 1:1543, 2:5392, 3:12453, 4:17574,
  5:17308, 6:10816, 7:4586, 8:1005, 9:252, 10:117.

## Validation

Atomic publication replayed and hash-checked every edge and verified canonical
keys, route invariants, parent-key superset and SQLite integrity. The B4 gate
preserved 286,334/286,334 inputs and independently replayed all 118,178 edges.
Runtime formatting, 61 tests, the frozen RF reference vector and strict Clippy
passed. Policy provenance remains frozen Q254 with objective L1000. No training,
RF Knots or pgx-mcts-bench write, commit, push or deployment occurred.

Artifacts:

- `unknotdb-descending-all-gt10-v0.sqlite`
- `unknotdb-descending-all-gt10-v0.tsv`
- `unknotdb-descending-all-gt10-v0-b4-regression.txt`
