# Graph identification sidecar v4/v5

The identification sidecar is proof-external. It maps both source-corpus
representation IDs and canonical proof-graph keys to stable knot IDs, and
provides the reverse postings relation `knot_representation_postings`.

Trust classes remain separate:

- `verified`: derived from an independently replayed UnknotDB fact;
- `attested`: provenance or an external Regina/SnapPy computation;
- `candidate`: a search hint that is physically excluded from effective maps.

Every replay-validated edge of CC cost zero proves knot equivalence. The
sidecar therefore treats the underlying undirected CC=0 component as one knot
type. A component is labelled only if all effective source seeds agree on one
knot ID; disagreements are quarantined in `cc0_component_conflicts`.

The bounded candidate rule is deliberately only necessary, not sufficient.
Two components become candidates when their active optimal routes use one CC
to the exact same target vertex and their determinant, Alexander polynomial,
Jones mirror orbit and absolute signature agree. Promotion requires a separate
external-equivalence record. External checks never create proof-graph edges.

Useful CLI queries are:

```text
unknotdb identification-for-graph KEY_OR_NODE
unknotdb representations-for-knot 3_1 --namespace graph
unknotdb list-equivalence-candidates --status pending
```
