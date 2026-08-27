# Planar U=1 certificate import for 9_19 and 9_21

## Result

The standalone Unknot DB graph now contains independently replayed U upper
bound 1 routes for both requested knots.  No exact lower-bound assertion is
introduced by this import.

| knot | source key | old U | new U | CC position | planar certificate SHA-256 |
|---|---|---:|---:|---:|---|
| 9_19 | `bddc6eee2516012432d919b5719d4a3a105c0b8a1e9cbef9319237cb48ddd64f` | 13 | 1 | 5 | `6ddb53a2367d9b5c7c6f092f83918fa4508896c39950122b0e16ad2c4f0b31c6` |
| 9_21 | `e2db5277945844b5c2578b3fe9febaea06ce6c09b74d8e01d06f5fba254b4df7` | 10 | 1 | 9 | `4112893a39db217b1b89636659800326b0dc4934f72df43dde33a3ec1bb703f0` |

Each route is stored as two graph edges:

1. one exact crossing change followed by the pinned mandatory preprocessing
   witness and normalization into a graph stopping point;
2. a zero-CC `PlanarCertificateCollapse` edge whose content-addressed sidecar
   contains the labelled RI/RII/RIII trace.

The Rust validator reconstructs the ordinary Artin braid closure, verifies the
input word, replays each local operation, checks every before/after SHA-256 and
crossing count, and requires the final state to be the empty diagram with one
unlinked unknot component.  Bare use of the planar instruction without its
sidecar is rejected by proof replay.

## Published artifacts

- Parent snapshot: `outputs/unknotdb-12a_1047-u2-v0.sqlite`, SHA-256
  `767127197c81117d2ae2e95ffe9b2785b4436d798493f14cc11b0686ff6a3490`.
- Intermediate 9_19 snapshot: `outputs/unknotdb-9_19-planar-u1-v0.sqlite`,
  SHA-256 `97f9cca12ac5911820260efc1bff06b630e2114268d3d6e0bdbd34bfb0e92003`.
- Final combined snapshot:
  `outputs/unknotdb-9_19-9_21-planar-u1-v0.sqlite`, SHA-256
  `c2d2dd17a792002372c7902ef53853f8677747a9ae1e2aaa19e80c56344421f3`.
- Per-import manifests: `outputs/unknotdb-9_19-planar-u1-v0.tsv` and
  `outputs/unknotdb-9_21-planar-u1-v0.tsv`.
- Source sidecars: `outputs/unknotdb-9_19-zero-reidemeister-trace-v0.json`
  and `outputs/unknotdb-9_21-zero-reidemeister-trace-v0.json`.

## Graph and storage delta

| metric | parent | final | delta |
|---|---:|---:|---:|
| nodes | 100,623 | 100,625 | +2 |
| edges | 153,599 | 153,603 | +4 |
| program templates | 17,235 | 17,236 | +1 |
| planar sidecars | 0 | 2 | +2 |
| snapshot bytes | 27,557,888 | 27,582,464 | +24,576 |

The shared planar program is deduplicated once; the two trace bodies are stored
separately by content hash.  The count of nodes with U greater than 10 changed
from 5,204 to 5,203, as expected from the direct 9_19 improvement.

## Validation

- Atomic writer publication and parent-key-superset check passed for both
  imports.
- Full Rust proof replay and all program/certificate hash checks passed while
  loading the final snapshot (153,603 edges).
- SQLite `PRAGMA integrity_check` returned `ok`.
- 64 Rust unit tests, the RF reference-vector test, doc tests, rustfmt check,
  and Clippy with `-D warnings` passed.
- The pinned policy identity remains
  `q-grown-raster-axial-12:Q254:36d1122f5494d502e556994083a1a69adf1643d5be95cd9f80ffc13b68e68d63`
  with the L1000 preprocessing contract.

RF Knots and pgx-mcts-bench were read-only.  No model was trained or changed,
and no commit, push, or deployment was performed.
