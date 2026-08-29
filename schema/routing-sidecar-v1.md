# Routing sidecar v1

The routing sidecar is a derived, immutable map pinned to one exact proof graph
SHA-256.  It does not add proof edges, change `U_upper`, or change the core
snapshot schema/version.

For an edge `e`:

- `cc_cost(e)` is the independently replayed value 0 or 1;
- `semantic_moves(e)` counts semantic `Action` instructions in the stored
  program; coordinate-only origin, mirror-orbit, and anchor instructions have
  zero semantic cost;
- `L1000(e) = 1000 * cc_cost(e) + semantic_moves(e)`.

The sidecar stores three independent route interpretations:

1. the core graph's active minimum-CC pointer;
2. the lexicographically shortest `(CC, semantic_moves)` route, equivalently
   the shortest semantic witness among routes with minimum known CC;
3. the scalar minimum of `1000*CC + semantic_moves`, which may use more CC.

`route_frontier` additionally records the shortest semantic route for exact CC
counts `U`, `U+1`, and `U+2` when such a directed route exists.  Missing rows
mean only that the current proof graph has no such route.

All distances are computed from every certified `U=0` vertex over reverse
adjacency.  Non-negative semantic edge weights use Dijkstra relaxation.
Deterministic equal-cost ties select the smallest edge ID.  The builder rejects
the sidecar unless its lexicographic CC distance agrees with core `U_upper` for
every vertex.

```bash
runtime/target/release/unknotdb-runtime build-routing-sidecar \
  proof.sqlite routing.sqlite

runtime/target/release/unknotdb-runtime show-routing \
  proof.sqlite routing.sqlite NODE_ID_OR_64_HEX_REP_KEY
```
