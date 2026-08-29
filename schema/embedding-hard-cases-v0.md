# Embedding hard-case sidecar v0

This immutable SQLite sidecar defines protected evaluation families for global
representation embeddings. It references, but does not mutate, an embedding
pair sidecar. Every representation belonging to a protected family is excluded
from metric training, even if the older generic split assigned it to train.

The first family is the replayed Wang--Zhang five-crossing cube for
`7_1 # mirror(7_1)`. Its 32 states form one protected test unit. The published
lower bound `u=5`, together with the replayed five-CC source-to-unknot path,
makes every comparable Boolean-cube subpath geodesic: replacing a length-one or
length-two subpath by a shorter route would shorten the full route below five.
These labels are marked `externally_attested_exact_geodesic`, not independently
proved lower bounds produced by UnknotDB itself.

```bash
python3 tools/build_embedding_hard_cases.py \
  --pairs outputs/unknotdb-embedding-pairs-v1.sqlite \
  --cube-manifest outputs/unknotdb-wang-zhang-five-crossing-cube-v0.tsv \
  --output outputs/unknotdb-embedding-hard-cases-v0.sqlite
```

The sidecar contains 32 protected representations, 80 exact CC-distance-one
pairs, and 80 exact CC-distance-two pairs. It is an evaluation artifact only;
the trainer must never sample these pairs as training examples.
