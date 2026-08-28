# Mixed proof-vertex contract v1

Normative for non-synthetic snapshots with `schema_version >= 4`.

## Key domains

Each graph vertex stores one braid encoding and exactly one key kind.

- `key_kind = 0` is a canonical value vertex. Its encoding must already equal
  the mirror/origin normalized representation and its key is the existing
  canonical `RepKey`.
- `key_kind = 1` is an exact proof checkpoint. Its encoding may be
  unnormalized. Its key is
  `SHA-256("UNKNOTDB_EXACT_BRAID_CHECKPOINT_V0\\0" || exact_v0_encoding)`.

The domains are distinct. Canonical lookup normalizes a query and searches only
the resulting canonical key. It cannot accidentally hit an exact checkpoint.
An exact checkpoint and its mirror remain distinct unless replayed proof edges
connect each to a common canonical vertex.

## Attestations

A canonical vertex may have one of three stopping attestations:

1. the pinned L1000 policy prefers an exact legal crossing change;
2. the representation is the terminal `B1 []`;
3. the deterministic decreasing reducer and normalization completed, but the
   pinned policy returned its exact strand- or word-capacity diagnostic.

The third case is `CapacityFallback`. It asserts no neural-network choice.
Errors other than the two pinned capacity diagnostics remain fatal.

A proof checkpoint may omit a stopping attestation. Its legitimacy comes from
the independently replayed incoming/outgoing proof programs, not from policy
inference. A checkpoint with no connected replay-validated route must not be
reported as a successful connected insertion.

## Edges and routes

All edge rules are unchanged: a stored program is replayed from the exact source
encoding, its CC cost is recomputed, and its exact output must equal the target
encoding. U is still shortest verified CC distance to a certified unknot.
Canonical and exact vertices may both participate in relaxation.

Compressed intermediate states remain preferred when no lookup, merge or audit
benefit justifies a vertex. Schema v4 permits exact checkpoints; it does not
require materializing every primitive state.
