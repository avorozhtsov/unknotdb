# Graph supervision dataset v1

The graph-supervision sidecar is a deterministic training-data projection of
one exact proof snapshot and one hash-pinned routing sidecar. It changes neither
the proof graph nor the frozen policy.

The exporter selects the independently derived `next_shortest_edge` for every
nonterminal node. It materializes each coordinate-independent program with the
edge's `(anchor_x, anchor_y)` binding, replays the complete program, and rejects
the export if a replayed endpoint differs from the stored target.

Each label contains the exact physical UKB0 representation immediately before
one semantic action. This state is intentionally not renormalized: the encoded
action position is meaningful in that exact torus chart. Coordinate-only
normalization and mirror instructions are replayed but do not become labels.

Two disjoint tasks are stored in `labels` and exposed as SQL views:

- `preprocessor_labels`: verified zero-CC `REDUCE`, `COMMUTE`, `BRAID`,
  `INSERT`, destabilization, and stabilization actions;
- `cc_solver_labels`: the exact `CROSSING_CHANGE` selected by a shortest
  semantic witness among currently known minimum-CC routes.

`preprocessor_stops` exposes the same exact states as `cc_solver_labels` but
without the CC action. These are positive termination examples: the zero-CC
preprocessor should stop there and hand the state to the CC solver. This avoids
inventing a fake proof primitive for `STOP`.

`PASS`, `DESCENDING_COLLAPSE`, and `PLANAR_CERTIFICATE_COLLAPSE` are not policy
labels. The latter two are theorem/certificate macros rather than primitive
network actions. Their exclusions are counted in the export report.

Exact duplicate `(task, state, action)` labels are deduplicated while every
source edge and instruction is retained in `occurrences`. A state may have more
than one independently verified action; these alternatives are preserved, not
arbitrarily collapsed. They share one split because splitting is determined by
the first byte of SHA-256 of the exact state encoding:

- train: `0..229`;
- validation: `230..242`;
- test: `243..255`.

This prevents the same physical state from leaking across splits. The dataset
is positive supervision from known upper-bound witnesses. It does not assert
that an action is uniquely optimal or prove a lower bound on unknotting number.

```bash
runtime/target/release/unknotdb-runtime export-graph-supervision \
  proof.sqlite routing.sqlite graph-supervision.sqlite

sqlite3 graph-supervision.sqlite \
  "SELECT task,split,count(*) FROM labels GROUP BY task,split"
```
