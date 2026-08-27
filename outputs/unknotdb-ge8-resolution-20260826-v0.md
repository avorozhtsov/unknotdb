# Investigation of every vertex with U upper bound at least 8

## Initial cohort

The input `unknotdb-descending-all-gt10-v0.sqlite` contained 1,374 vertices
with U >= 8: 1,005 at U=8, 252 at U=9 and 117 at U=10.

All 1,374 were first reattested and followed under the frozen Q254 L1000
top-1 policy. This directly improved 85 selected vertices and 66 vertices
collaterally. A second identical pass reached a fixed point with zero new
improvements.

The remaining cohort was then searched with exact bounded branching. At every
branch the algorithm performs exactly one CC, runs the mandatory pinned Q254
preprocessing contract, normalizes the successor and independently replays the
program. A chain is persisted only when it terminates at an exact key already
connected to a finite-U graph route. Intermediate raw states are never vertices;
only their normalized preprocessed stopping points are inserted.

Depths 1 through 5 evaluated 8,212,944 branches and found 488,918 exact graph
hits. Branching directly improved 1,223 selected vertices and added 1,255
immutable edges. Together with policy tracks, the campaign added 1,348 edges,
29 normalized stopping-point nodes and 189 deduplicated programs.

## Result

The final snapshot `unknotdb-branch-depth5-ge8-v0.sqlite` has 71,188 nodes,
119,526 edges and 10,237 programs in 18,300,928 bytes. U[p50/p95/max] is
4/7/9. Of the original 1,374 vertices, 1,367 now have U <= 7.

Seven vertices remain exact bounded misses through depth 5. Each has a valid
descending upper bound, but no checked branch of at most five CC macros reached
a graph target giving a strict improvement:

- U=9, word length 18: `0710f71545d94c99dbe140b7c9b7298bbdd42f8f204135e4b8e4af7287de4ba1`
- U=9, word length 18: `4dc995f3ac9c5af6c28d24c60a2c9e920fd7beebac1f3220b685c81130dd99c4`
- U=8, word length 16: `15159234843c5a7cd502b376b184fa9193355e551f63a55c7d471e4e4cad117c`
- U=8, word length 18: `2f774de6b2152aa226fa685fbe62c72b9810e8f3b76a62b247a4527afde2825c`
- U=8, word length 16: `b24614e42a3f4ae6423cbb75c8f1596c083ceef8a9869db842873cb1aefe2dd7`
- U=8, word length 18: `6bcf8db3772c9a4ad5241fd334011eef786f4e550ccc800169718022ba258866`
- U=8, word length 18: `b11500dbb91e69988ec50f617860f13f9c8c6def864c424a1994c8a99cf44d09`

The depth-5 pass alone evaluated 6,806,954 branches and 344,155 exact graph
hits for these seven without a strict improvement. This is not a claim that
their true unknotting numbers are 8 or 9. It is a precise exhaustion statement
for this graph, preprocessing contract and bounded search depth. Exhaustive
depth 6 would require tens of millions more branches; a targeted MCTS or a
stronger compatible policy is a better next experiment.

## Validation

Atomic publication replayed and hash-checked all 119,526 edges. SQLite
integrity and foreign keys passed. The protected B4 gate retained
286,334/286,334 coverage. Runtime formatting, 61 tests, the frozen RF reference
vector and strict Clippy passed. No training, RF/pgx write, commit, push or
deployment occurred.

Artifacts include every pass manifest from `unknotdb-policy-all-ge8-v0.tsv`
through `unknotdb-branch-depth5-ge8-v0.tsv`, the final snapshot, and
`unknotdb-branch-depth5-ge8-v0-b4-regression.txt`.
