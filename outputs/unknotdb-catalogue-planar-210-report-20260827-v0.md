# Catalogue planar witness import: 210 graph improvements

Date: 2026-08-27

## Outcome

All 210 previously identified strict graph improvements now have constructive,
independently replayed routes in Unknot DB:

- 2 were published first for `9_19` and `9_21`;
- the batch run published the remaining 208/208 with zero validation failures;
- all 233 catalogue witness entries whose exact standard-braid key is present
  in the graph now have `U_upper <= catalogue witness cost`;
- `8_19`, `10_1`, and `10_3` remain representation-key misses and were not
  counted among the 210 exact-key improvements.

The graph stores constructive upper bounds only.  Catalogue lower-bound claims
remain provenance data rather than proof edges.

## Chain contract

For crossing positions `p_1,...,p_k`, the importer constructs:

1. a normalized/preprocessed source stopping point;
2. one proof edge containing exactly one CC at `p_i`, followed by the pinned
   L1000 preprocessing witness and deterministic normalization;
3. for `i > 1`, an exact zero-CC inverse of the preceding preprocessing before
   applying `p_i`, preserving the coordinates of the original planar witness;
4. after the final stopping point, an exact inverse preprocessing prefix and a
   versioned `PlanarCertificateCollapse` instruction to canonical `B1 []`.

Every intermediate post-CC state is therefore represented only by its current
mandatory preprocessed/normalized stopping point.  Raw successors are replay
checkpoints, not graph vertices.

The Rust RIII implementation replays Spherogram's six temporary two-ended
strands literally.  This matters for degenerate local adjacencies, where a
direct border-rewiring shortcut is not equivalent.

## Measured import

Batch input contained 239 catalogue entries and 236 replayed witnesses:

| disposition | count |
|---|---:|
| strict improvements in this batch | 208 |
| already sufficient, including `9_19` and `9_21` | 25 |
| source standard-braid key absent | 3 |
| validation failures | 0 |

The 208 new routes comprise 65 one-CC, 120 two-CC, and 23 three-CC
certificates.  Their direct aggregate U decrease is 1,139, with individual
decreases from 1 through 17.  Bellman propagation improved 340 additional
existing vertices.  Including the earlier two imports, total U mass fell by
1,839: 1,160 direct and 679 collateral.

## Graph and storage delta

Relative to `outputs/unknotdb-12a_1047-u2-v0.sqlite`, before any planar import:

| metric | before | final | delta |
|---|---:|---:|---:|
| nodes | 100,623 | 100,901 | +278 |
| edges | 153,599 | 154,162 | +563 |
| program templates | 17,235 | 17,345 | +110 |
| distinct planar sidecars | 0 | 207 | +207 |
| snapshot bytes | 27,557,888 | 28,663,808 | +1,105,920 |
| sum of U upper bounds | 494,478 | 492,639 | -1,839 |
| nodes with U > 10 | 5,204 | 5,170 | -34 |
| nodes with U >= 8 | 10,553 | 10,477 | -76 |

There are fewer than 210 distinct sidecar blobs because content-addressing
deduplicates identical planar traces.  The program dictionary similarly stores
only 110 additional templates for 563 new edges.

## Artifacts

- Final immutable snapshot:
  `outputs/unknotdb-catalogue-planar-210-v3.sqlite`
- Snapshot SHA-256:
  `b3923e86157cfe16970bd6c079e152508086b6e5945a5a29d6e97e4df2f8eecb`
- Durable batch manifest:
  `outputs/unknotdb-catalogue-planar-210-v3.tsv`
- Manifest SHA-256:
  `f0f52be16cd534e9613043ab91fb98999acb2dfdd5a4843621cff097571b9204`
- Earlier two-route manifest/report:
  `outputs/unknotdb-9_19-9_21-planar-u1-import-20260827-v0.md`
- Frozen witness corpora SHA-256:
  `f3620f939bfc1ebca0d1568bb28bdc5aba8dde46d3e2b0d367f0d0a2ad31998b`
  and `70bc3c76816ec9f8a90343e835d24ca1c5cdd6d7104993eb068bbe68edcaac10`.

## Validation

- Atomic publication and parent-key-superset checks passed.
- Complete Rust replay, program hashes, sidecar hashes, and route invariants
  passed for all 154,162 edges.
- SQLite `PRAGMA integrity_check` returned `ok`.
- Protected B4 coverage is preserved transitively through the immutable parent
  snapshot and the exact key-superset gate.
- 65 Rust unit tests, RF reference vectors, doc tests, rustfmt, and strict
  Clippy with `-D warnings` passed.
- Policy provenance remains frozen Q254 with objective L1000 and adapter v4.

RF Knots and pgx-mcts-bench were read-only.  No training, checkpoint change,
commit, push, or deployment was performed.
