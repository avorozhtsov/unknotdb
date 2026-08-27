# Unknot DB lookup maps v1

- Sidecar: `outputs/unknotdb-lookup-maps-v1.sqlite` (8,708,096 bytes)
- Source representations: 3,519
- `representation → knot_id`: 3,415
- `canonical graph vertex → knot_id`: 3,374
- Stable knot IDs: 3,401
- Deduplicated knot-level invariant values: 23,276
- Effective `representation → invariant` rows exposed by the view: 23,374
- Cross-representation invariant conflicts: 0
- Representation-specific overrides: 0

Large polynomial values are stored once per `knot_id`; `representation_invariant_map` is a view, so it behaves like the requested map without copying Alexander/Jones data for every representation. Values are promoted to knot scope only when all available provenance-bearing values agree across mapped source representations. Signatures and missing Alexander/determinant values were recomputed; bundled Alexander/Jones/determinant values retain their table provenance. Any future disagreement is retained in `invariant_conflicts` and `representation_invariant_overrides` rather than overwritten.

The reverse maps `invariant_knot_map` and `invariant_representation_map` use the indexed posting key `(invariant_id,value_text)`. Multi-invariant lookup intersects postings at query time; no quadratic table of every invariant pair is stored.

## Lookup examples

```sql
SELECT knot_id FROM representation_knot_map WHERE representation_id=?1;
SELECT lower(hex(stopping_key)),graph_node_id FROM representation_graph_map WHERE representation_id=?1;
SELECT invariant_id,value_text FROM representation_invariant_map WHERE representation_id=?1;
SELECT knot_id FROM graph_vertex_knot_map WHERE stopping_key=?1;
SELECT knot_id FROM invariant_knot_map WHERE invariant_id=?1 AND value_text=?2;
```

This layer is metadata-only and snapshot-pinned. It cannot modify proof edges, `U_upper`, or the independent validator's conclusions.
