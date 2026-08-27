# RF-targeted Unknot DB population report, 2026-08-26

## Measured result

The versioned RF corpus has 2,989 unique parseable representations. Q254's
declared capacity accepts 2,987; 2 are excluded for word length greater than
48. Of the compatible inputs, 2,986 reached a valid policy stopping point and
one did not. Coverage was 188 before this run and 208 after forced insertion;
2,778 compatible stopping points remain missing.

Forced connected insertion attempted the first 20 deterministic priority-2
misses after the already investigated 108 priority-0/1 misses. It connected
20/20, added 28 nodes and 28 edges, and used 76 of the allowed 5,000
simulations. No disconnected node was published.

The optimizer cohort contains exactly 2,000 RF representations. Its immutable
selection rule is: connected inputs first by descending U, then ascending
priority, representation id and stopping key; fill with misses in that same
stable order. It contains 208 connected representations and 1,792 misses,
collapsing to 189 unique connected canonical nodes. The optimizer used 3,024
of 3,328 simulations and produced 5 strict direct improvements:

| RF representation | old U | new U |
|---|---:|---:|
| `braid:667113...20c4` | 812 | 409 |
| `braid:853414...cdb3` | 7 | 6 |
| `braid:73dbce...eb0` | 7 | 6 |
| `braid:982acb...4488` | 7 | 5 |
| `braid:24f8a2...b8aa` | 5 | 4 |

The other 184 unique connected targets had no improvement within budget.
Bellman relaxation retained all collateral changes and reduced aggregate U by
82,220. Final graph counts are 68,902 nodes, 69,698 edges and 1,843 deduplicated
programs in 13,197,312 bytes. Relative to the pre-run high-U snapshot this is
+28 nodes, +33 edges and +6 programs.

## Validation

The final snapshot SHA-256 is
`2f84050b8768677060d45d61f1d6158de3e573161c12244142d54a15287b3ad7`.
SQLite integrity is `ok`; publication independently replayed every accepted
edge and checked program hashes. The final B4 gate replayed all 69,698 edges,
retained 286,334/286,334 frozen enumeration coverage and reports U p50/p95/max
7/1160/1888. Atomic publication verified that every parent key remained.
The original large completion manifest is not present in this checkout; this
gate therefore uses a regenerated count assertion plus the fixed enumerator.
The protected corpus identity remains the previously recorded manifest SHA-256
`7a9ba50b...d622cc`, while the new gate's assertion-file SHA is intentionally
different. Coverage preservation is additionally enforced by the atomic
parent-key-superset chain.

Runtime formatting, 59 unit tests, the frozen RF vector test, and strict Clippy
passed. Verifier tests passed (31 unit and 4 API); verifier strict Clippy remains
red on five pre-existing lints in unchanged verifier code. Ruff and Python
bytecode checks passed for all touched tools.

No training, RF Knots/pgx-mcts-bench write, commit, push, or deployment occurred.

## Artifacts

- `unknotdb-rf-optimizer-2000-v0.sqlite`: final immutable snapshot.
- `unknotdb-rf-optimizer-2000-v0.tsv`: complete optimizer audit/manifests.
- `rf-knots-optimizer-cohort-2000-v0-selection.tsv`: exact 2,000-row order.
- `unknotdb-rf-full-connected-v0.tsv`: forced-insertion audit/manifests.
- `unknotdb-rf-optimizer-2000-b4-regression-v0.txt`: replay/B4 gate.
- `unknotdb-policy-selection-comparison-20260826-v0.md`: checkpoint investigation.
