# Federated catalogue and crossing-adjacency sidecar v1

The federated sidecar is a read-only search layer beside the immutable proof
graph.  It covers public catalogue identifiers without creating one graph node
per public diagram.  Every file is pinned by source URL, retrieval date and
SHA-256.

## Trust boundary

The three adjacency statuses are intentionally different:

- `claimed`: a source states a knot-type crossing-change relation;
- `diagram_attested`: the source supplies a fixed diagram and crossing locator;
- `replay_verified`: the row names a one-CC edge in a pinned Unknot DB graph
  snapshot, with both canonical stopping keys and the graph edge ID.

Only the last status projects a proof edge.  Neither a KnotInfo property nor an
external adjacency claim changes `U_upper`, `next_unknot`, or ACS10.  Adjacency
is source-bounded; absence is never a claim that two knots are not Gordian
neighbors.

## Physical layout

- `catalogue_sources`: immutable provenance and capability boundary;
- `knots` and `knot_identifiers`: dense local IDs plus external aliases;
- `representations`: DT, Gauss, PD and braid data, deduplicated by SHA-256;
- `property_definitions`, `property_values`, `knot_properties`: compact forward
  and reverse maps for selected hot catalogue fields;
- `graph_knot_links`: snapshot-local graph node IDs joined by durable stopping
  keys;
- `adjacency_claims`: typed, indexed CC claims with optional diagram binding or
  exact proof-graph binding.

The original catalogue snapshot is the cold source for fields not copied into
the hot sidecar.  This prevents large specialist polynomial tables from being
duplicated merely to support name, representation, invariant and adjacency
queries.

### Brittenham distance-one import

The Brittenham sliced archives enumerate all sign subsets of one standard
alternating projection for each `12a_*` and `13a_*` knot.  The importer streams
the ZIP members and retains exactly the rows whose DT sign vector has Hamming
distance one from the all-positive source, modulo global sign reversal.  Every
claim stores the source projection, changed DT entry, raw successor, identified
target and exact archive line.  The companion SnapPy identification output is
an independently hash-pinned source for `13n_*` target names.

This is complete adjacency for those particular standard projections, not the
complete Gordian neighborhood of their knot types.  Connected-sum targets are
counted but omitted until the catalogue has canonical composite-knot IDs.

## Identifier and representation conventions

KnotInfo names are canonical (`12n_570`).  Spherogram spelling (`K12n570`) and
DT/Conway names are aliases, never alternate canonical IDs.  Representation
hashes cover `encoding`, a zero byte, exact UTF-8 source text, and a final zero
byte.  No imported diagram is automatically a proof-graph vertex.

## Publication checks

A sidecar is published atomically only after SQLite `integrity_check`,
`foreign_key_check`, deterministic counts and a manifest containing all input
hashes.  The CLI opens it read-only and rejects unknown schema versions.
