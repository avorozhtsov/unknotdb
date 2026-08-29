# Embedding pair sidecar v0

This derived SQLite sidecar is an immutable, hash-pinned source of interval
supervision for full-representation embeddings. It changes neither the proof
graph nor a policy checkpoint.

The current SQLite schema identifier is `unknotdb-embedding-pairs-v3`.
`representation_domains` distinguishes graph stopping points from exact states
replayed inside edge programs and records states produced by mirror-orbit
instructions. A representation may have more than one role. Consumers should
use this table to measure and control stopping-point distribution bias; split
membership remains assigned to the complete exact-CC=0/attested-knot component.

It stores two metrics:

- `cc`: semantic crossing-change distance in UnknotDB's mirror-orbit task
  quotient;
- `rm`: origin-quotiented reversible braid Reidemeister/Markov primitive
  distance. Mirror is deliberately excluded from this metric.

Every pair carries `[distance_lower,distance_upper]`. A replayed path supplies
an upper bound, not automatically an exact distance. Exact RM distances zero
and one follow from coordinate/primitive replay. A two-primitive witness is
labelled exact two only after exhaustive enumeration proves that no permitted
single zero-CC primitive reaches the target modulo cyclic origin.

The exporter independently materializes and replays every supported braid
edge. It excludes planar theorem macros from this first braid-only dataset.
Exact CC=0 relations and attested knot IDs are unioned into split units. The
unit hash selects train/validation/test; a pair crossing split boundaries is
discarded. This prevents representations of the same known knot/orbit from
leaking between ordinary metric splits.

```bash
runtime/target/release/unknotdb-runtime export-embedding-pairs \
  proof.sqlite identification.sqlite embedding-pairs.sqlite
```

The sidecar does not claim that uncertain `[0,1]`, `[0,2]`, or `[1,2]` pairs
have exact distance equal to their upper bound.
