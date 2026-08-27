# Unknot DB policy-selection comparison, 2026-08-26

## Frozen baseline

The executable baseline is `q-grown-raster-axial-12:Q254:36d1122f...68e68d63`,
objective ratio L1000, frozen from the Q254 `raster-axial-12` state on
2026-08-24. The checkpoint SHA-256 is
`36d1122f5494d502e556994083a1a69adf1643d5be95cd9f80ffc13b68e68d63`;
its source-state SHA-256 is
`5cb60218a2665482c0b2695cc98ee79cb17c6f4c94ce8d14f1101a460bfdb0aa`.
The model supports the current 12-strand raster observation and semantic action
contract and is loaded successfully by `tools/q_policy_oracle.py`.

On the fixed compatible RF corpus, Q254 preprocessing produced 2,986 complete
stopping points from 2,987 inputs and initially hit 188 graph routes (6.30%).
The bounded priority-2 forced-insertion block then connected 20/20 cases using
76 simulations. Greedy policy-chain and graph-assisted search both found all
20; graph assistance improved the resulting bound in 4 cases, tied in 16, and
lost none. Greedy/graph-assisted U pairs were: 5/5, 5/5, 5/5, 6/6, ten 7/7
or better cases, and the remaining values shown exactly in
`unknotdb-rf-full-connected-v0.tsv`; aggregate search depths were 37 and 39.
All 28 accepted edges (including exact inverse witnesses where valid) replayed.
No replay-validation failure occurred. Observed interactive wall time was about
6 seconds; the v0 manifest records simulations but not wall time.

The deterministic RF optimizer cohort contains exactly 2,000 representations:
208 current exact hits first by descending U/priority/id, followed by 1,792
misses. Canonical deduplication yielded 189 optimizer targets. Under 16
simulations per target, 5 improved and 184 did not; 3,024 simulations added 5
validated edges. The direct U changes were 812->409, 7->6, 7->6, 7->5 and
5->4. Bellman collateral relaxation reduced aggregate U by 82,220. Observed
interactive wall time was about 17 seconds. The 1,792 disconnected cohort rows
were reported as misses and were not inserted or asserted.

## Candidate inspection

No genuinely newer, stronger, executable checkpoint was found locally.

- The registered Q304 continuation is only a protocol/audit. Its directory has
  zero `.pt`/`.pt.gz` model files, and its preparation log terminates because
  the Q254 terminal marker was missing. It cannot be loaded or compared.
- The native strand-12 Q20 raster checkpoints are older than Q254 and have no
  evidence here that they are stronger. They are not replacement candidates.
- Other Q254 branch architectures are not newer than the baseline, are not
  frozen under the Unknot DB loader/provenance contract, and some require a
  different model class. Treating them as drop-in checkpoints would not be an
  apples-to-apples comparison.
- Archived pre-semantic-moves rung-18 checkpoints use an earlier action/state
  contract and are incompatible with current replay semantics.

Because the compatible candidate set is empty, no alternative-policy snapshot
was prepared or published and there are no candidate gains/losses to report.
The factual recommendation is to retain frozen Q254. Revisit Q304 only after an
immutable checkpoint, complete source-state/action metadata, and a working
loader exist; then run offline reattestation on this same corpus and publish to
a separate atomic snapshot only if protected B4/RF coverage does not regress.

RF Knots and pgx-mcts-bench were inspected read-only. No training or checkpoint
mutation occurred.
