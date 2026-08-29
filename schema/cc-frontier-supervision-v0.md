# CC frontier supervision v0

This derived SQLite sidecar provides conservative set-valued training targets
for a crossing-change policy. It is pinned by SHA-256 to one proof snapshot and
one routing sidecar and does not mutate either.

The exporter examines every stored edge of CC cost one whose target has a
finite derived route. It materializes the anchored program, independently
replays the complete edge, and records the exact physical UKB0 state and
semantic CC action at the unique crossing-change instruction. The complete
candidate cost is

`1 + U(target)` with the full edge and suffix semantic length retained for
audit.

Options are grouped by exact state. `accepted=1` means that the complete route
matches the source node's current minimum graph CC distance. Other stored
options are replayed, completed comparisons. Actions absent from `options` are
unknown and must receive zero graph-batch gradient; they are not negatives.
Exact `(state, action)` duplicates are collapsed to the cheapest deterministic
occurrence.

The state-hash split is identical to graph supervision v1. The metadata pins a
native-controller allowance of at most five nonsemantic actions before the
semantic CC. Native compilation must abstain when that bound is exceeded.

```bash
runtime/target/release/unknotdb-runtime export-cc-frontier-supervision \
  proof.sqlite routing.sqlite cc-frontier.sqlite
```

This dataset identifies the best result observed in the current verified
graph. It neither proves uniqueness nor supplies a lower bound on the true
unknotting number.
