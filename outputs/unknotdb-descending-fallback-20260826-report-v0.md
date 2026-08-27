# Descending fallback backfill, 2026-08-26

## Verified facts

The input was `unknotdb-rf-optimizer-2000-v0.sqlite` (SHA-256
`2f84050b8768677060d45d61f1d6158de3e573161c12244142d54a15287b3ad7`):
68,902 nodes, 69,698 edges, 1,843 programs, 13,197,312 bytes, and
U[p50/p95/max] = 7/1160/1888.

The final snapshot is `unknotdb-descending-rf64-v2.sqlite` (SHA-256
`3bc3ec0f07c6bfe4f2b533b0f15278c4f7d97a8859becbaaefeb7932e12813a8`):
69,043 nodes, 70,206 edges, 2,199 programs, 13,291,520 bytes, and
U[p50/p95/max] = 7/1007/1883. Nodes with U >= 1000 fell from 7,258 to
4,973; nodes with U >= 1800 fell from 835 to 136. The snapshot grew by
94,208 bytes (0.71%).

The first cohort was the 32 highest-U eligible ordinary-Artin knot closures,
threshold U >= 1886 and word length <= 96. It included all nine original
max-U=1888 nodes. All 32 certificates validated, directly improving 32 nodes
and collaterally improving 670 more; it inserted 5 nodes and 98 edges.

The bounded RF cohort was the first 64 exact connected keys from the versioned
RF audit after deterministic ranking by descending U, then strands, word
length and key. All 64 certificates validated. It directly improved 59 selected
nodes and collaterally improved 4,408 nodes; it inserted 136 nodes and 410
edges relative to the post-high-U snapshot. No representation was invented and
RF Knots was read-only.

The two highlighted four-strand, 15-letter vertices both changed from U=1888
to U=8:

- `707a9ce0fd33cc391fd9ca348c3a1a31afe2158f51ecc792100b7a0decb6bf3a`
- `7b2615a621cde3b36176fda0981cac6c228a1005eb575b3b86eb22044c1817d2`

## Exact certificate convention

The v0 traversal starts at top strand 0 and follows the oriented ordinary
Artin closure downward, with closure bottom position i connected to top
position i. At positive sigma_i the branch entering on the left is over; at
negative sigma_i the branch entering on the right is over. A crossing is
changed exactly when it is first encountered under. The terminal theorem macro
independently checks existence of a cyclic base point for which every crossing
is first encountered over, or every crossing is first encountered under (the
ascending mirror). Either monotone closure is an unknot.

Every individual CC has its own graph edge. Its raw successor is never stored.
The raw successor is passed through the pinned mandatory reducer plus frozen
Q254 L1000 policy preprocessing and deterministic mirror/origin normalization.
The complete preprocessing witness is stored in the edge program, and only its
normalized stopping point is inserted/deduplicated as the next vertex. Later
edges exactly invert the prior preprocessing witness, perform one CC, and
preprocess again. A final zero-CC monotone-collapse edge reaches canonical B1.

The route recomputation computes U as reverse-graph 0-1 BFS from canonical B1,
then performs a deterministic tight-edge rank/pointer pass for lexicographic
ties and cycle safety. The separate incremental path propagates strict
improvements over incoming edges with a deque; its result is tested against a
full recomputation. This population run conservatively performed complete
recomputation and an independent full replay before atomic publication.

## Validation

- Runtime formatting passed; 61 unit tests and the frozen RF reference-vector
  test passed; strict runtime Clippy passed.
- Verifier formatting, 31 unit tests and 4 public-API tests passed. Strict
  verifier Clippy still reports the same five pre-existing warnings in
  unchanged verifier sources.
- The publication path replayed all 70,206 edge programs and checked hashes,
  canonical keys, route invariants, SQLite integrity and parent-key superset.
- SQLite `integrity_check` returned `ok`; `foreign_key_check` returned no rows.
- The B4 gate preserved 286,334/286,334 preprocessed seeds and replayed all
  70,206 edges.
- Frozen policy provenance remained
  `q-grown-raster-axial-12:Q254:36d1122f5494d502e556994083a1a69adf1643d5be95cd9f80ffc13b68e68d63`
  with objective L1000. There was no training, RF/pgx write, commit, push or
  deployment.

## Limitations and next bounded target

The generator currently supports one-component closures of ordinary Artin
braids only; cyclic band-generator inputs are excluded. The run was deliberately
bounded, so max U remains 1883 on unprocessed vertices. The next evidence-based
step is another durable high-U cohort, not an unbounded census.

## Artifacts

- `unknotdb-descending-high-u-v1.sqlite` and `.tsv`: first 32-node high-U run.
- `unknotdb-descending-rf64-v2.sqlite` and `.tsv`: final RF64 snapshot and
  per-certificate manifest.
- `unknotdb-descending-rf64-v2-b4-regression.txt`: complete replay/B4 gate.
- `unknotdb-rf-postconnect-audit-v0.tsv`: versioned RF source audit.
