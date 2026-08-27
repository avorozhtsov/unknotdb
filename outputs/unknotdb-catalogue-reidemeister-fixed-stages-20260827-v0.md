# Fixed catalogue Reidemeister witness stages

Date: 2026-08-27

## Frozen inputs

- Graph: `outputs/unknotdb-12a_1047-u2-v0.sqlite`
- Graph SHA-256:
  `767127197c81117d2ae2e95ffe9b2785b4436d798493f14cc11b0686ff6a3490`
- Catalogue sidecar: `outputs/unknotdb-catalogue-u-v3.sqlite`
- Catalogue provenance: KnotInfo 2026-08-14, as recorded in that sidecar.
- Witness representation: the mirror/origin-normalized Spherogram standard
  Artin braid for each named prime knot.
- Search: exhaustive lexicographic subsets of exactly the catalogue `U`
  crossings, followed by exact RI/RII/RIII simplification. A fixed seeded-level
  fallback with seed 0 was used only after the lexicographic scheduler had
  exhausted every crossing subset.
- Validator: reconstruct the post-CC braid closure and replay only the recorded
  local moves, checking labelled planar-state SHA-256 before and after every
  move. Validation never calls `Link.simplify`.
- Engine: SnapPy 3.3.2 and Spherogram 2.4.1.

Catalogue `U` is used only to fix the number of CCs to enumerate. Acceptance is
based on the independently replayed constructive upper-bound witness, not on
the catalogue assertion.

## Stage 1: all prime knots with crossing number at most 9

- Selected knot types: 84/84.
- Witnesses: 84/84; misses: 0.
- Exact standard-braid graph-key hits: 83/84.
- The sole non-hit is `8_19`; its witness exists, but the standard braid must
  still pass the pinned preprocessing contract before it can attach to the
  existing graph stopping point.
- Witness counts by exact `U`: `U=1`: 35, `U=2`: 37, `U=3`: 11, `U=4`: 1.
- Zero-CC moves replayed: RI 265, RII 447, RIII 82.
- Of the exact graph-key hits, 70 witnesses would strictly improve the current
  graph bound; 13 already have the catalogue bound at that exact vertex.
- Aggregate prospective decrease across those 70 vertices: 348 CC; largest
  individual gap: 14.

Artifacts:

- `outputs/unknotdb-catalogue-reidemeister-c3-c9-v0.json`
- `outputs/unknotdb-catalogue-reidemeister-c3-c9-v0.tsv`
- JSON SHA-256:
  `f3620f939bfc1ebca0d1568bb28bdc5aba8dde46d3e2b0d367f0d0a2ad31998b`
- TSV SHA-256:
  `61bed9355bf8afa055aa0935028fb45740cd8465005017eb30b43f823aa64c3e`

## Stage 2: 10-crossing knots with exact catalogue U

- Selected knot types: 155.
- Witnesses: 152; truthful misses: 3.
- Exact standard-braid graph-key hits: 153/155. The two catalogue types whose
  standard braid is not currently a graph key are `10_1` and `10_3`; both have
  replayed witnesses.
- Witness counts by exact `U`: `U=1`: 42, `U=2`: 93, `U=3`: 14, `U=4`: 3.
- Zero-CC moves replayed: RI 514, RII 1,012, RIII 236.
- Of the graph-key hits with witnesses, 140 would strictly improve the current
  graph bound; 10 already match it.
- Aggregate prospective decrease across those 140 vertices: 812 CC; largest
  individual gap: 17.

Remaining misses after exhaustive exact-CC enumeration under the fixed local
scheduler contract:

| Knot | Exact catalogue U | Current graph U | Result |
|---|---:|---:|---|
| `10_27` | 1 | 6 | CC position 15 reaches an unknot under Spherogram `pickup`; a pure recorded RI/RII/RIII trace was not obtained |
| `10_162` | 2 | 5 | no accepted exact-2-CC local trace in the fixed budget |
| `10_164` | 1 | 6 | no accepted exact-1-CC local trace in the fixed budget |

Artifacts:

- `outputs/unknotdb-catalogue-reidemeister-c10-exact-v1.json`
- `outputs/unknotdb-catalogue-reidemeister-c10-exact-v1.tsv`
- JSON SHA-256:
  `70bc3c76816ec9f8a90343e835d24ca1c5cdd6d7104993eb068bbe68edcaac10`
- TSV SHA-256:
  `d4f923ea978c832bd7a65733228c5ab93e1bbde0d11cb86c5562f505ff75ae0d`

## Validation and publication boundary

- Fresh full replay: 84/84 stage-1 witnesses and 152/152 stage-2 witnesses.
- SQLite source snapshot integrity: `ok`.
- Python byte-code compilation: passed.
- Ruff: passed.
- Rust tests: 64/64 passed (63 unit plus one RF reference-vector test).
- `cargo fmt --check`: passed.
- strict Clippy with `-D warnings`: passed.

No graph snapshot was published in this stage. The current Rust proof-program
codec is braid-native and cannot yet bind a labelled planar Reidemeister trace
to an edge. Assigning the prospective bounds before adding that verifier would
turn checked certificates into unverified assertions. The next implementation
step is the versioned planar-certificate instruction/sidecar and Rust replay;
then each multi-CC witness must be split into zero/one-CC edges with the pinned
preprocessing and normalization applied at every intermediate vertex.

RF Knots and pgx-mcts-bench were not modified. No policy was trained or changed.
No commit, push, or deployment was performed.
