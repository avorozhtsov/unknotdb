# Preprocessing contract v3

Normative contract for producing a graph lookup key and for deciding whether a
normalized braid may be stored as a production vertex.

## Fixed inputs

A preprocessing run fixes all of the following before it starts:

- representation codec and normalizer version;
- reducer version `l10-ri-r2-decreasing-v0`;
- policy adapter
  `initial-reducer-clean-controller-top1-self-loop-skip4-mirror-orbit-v3`;
- one immutable policy checkpoint identified by its SHA-256 model ID;
- objective `L1000`;
- controller initial state `canonical-clean-v0`;
- positive total-policy-ply and semantic-move bounds.

Changing any item creates a different preprocessing contract. A production
snapshot contains exactly one such contract; model versions must not be mixed.

## Initial-reducer pipeline

Normalization is always the first operation. The complete top-1 v0 route is:

```text
MirrorOrbitNormalize(input)
  -> decreasing RI/RII reducer
  -> Normalize
  -> start policy controller from canonical clean state
  -> follow internal controller actions until first semantic preference
       CC       -> stop before applying CC; graph lookup is permitted
       zero-CC  -> replay it independently; normalize only; start a clean policy call
       anything else / bound / cycle -> fail this candidate
```

The decreasing reducer runs exactly once, before the first policy decision. It
does **not** run after a policy semantic move and there is no final reducer after
the policy prefers CC. A policy may intentionally expand a representation to
expose a better crossing change; post-policy simplification could destroy that
prepared state. After every accepted zero-CC policy move, only mirror/origin
normalization is applied before the next clean policy call. Head, tape and
memory state are never part of a graph key.

The canonical unknot `B1 []` is the sole terminal exception and needs no CC
preference. A nonterminal output is a graph stopping point if and only if the
first semantic preference from the fixed clean-controller/L1000 contract is a
legal crossing change. `PASS`, environment termination, controller or
representation cycles, and exhausted bounds are incomplete preprocessing, not
stopping points. They must not be looked up or inserted as production vertices.
A stopping point need not be an RI/RII fixed point: reducibility created
deliberately by a policy route is retained until its preferred CC is applied.

The model is heuristic. Rust independently decodes, checks and applies every
returned semantic action. The model process may apply only internal controller
actions. The accepted zero-CC route and all reducer moves are replayable;
mirror-boundary instructions are recorded separately and rely only on the exact
symmetry `U(K)=U(mirror(K))`.

The v3 adapter also rejects a proposed zero-CC move when applying and
normalizing it returns a representation key already seen in the current
preprocessing route. It then asks the same clean-controller decision for the
next-ranked semantic action with that exact action excluded. At most four
distinct self-loop/cycle actions may be excluded at one representation; the
fifth failure remains an incomplete `RepresentationCycle`. Rejected proposals
are neither applied nor written to the proof program. This bounded exclusion is
deterministic, replay-neutral and prevents a quotient-normalization self-loop
from hiding the next policy preference.

## Decreasing reducer

For a braid representation define

```text
L10 = 10 * strands + word_length.
```

The reducer uses only:

- `DESTABILIZE`, the decreasing Markov/R1 move, reducing L10 by 11;
- `REDUCE(p)`, cyclic free reduction/R2, reducing L10 by 2.

Scheduling is deterministic:

1. normalize the input word origin;
2. if `DESTABILIZE` is legal, apply it;
3. otherwise scan cyclic positions from zero and apply the first legal
   `REDUCE(p)`;
4. repeat until neither move is legal;
5. normalize the output origin.

The schedule prefers the greatest immediate L10 decrease. After each primitive,
the implementation asserts strict decrease, so termination follows from the
non-negative integer measure L10. The output includes the zero-CC program and
input/output normalization witnesses.

R3/BRAID, COMMUTE, INSERT, STABILIZE and inverse/expanding RI/RII moves are not
part of the mandatory reducer. A policy may select a permitted zero-CC semantic
move, including one of these, only inside the declared bounds and only when the
independent Rust semantics can replay it.

## Policy-stop attestation

Every production vertex has one `policy_stops` row. It records either:

- `PreferredCrossingChange(action, audit_sha256)`, where `action` is a legal CC
  in that vertex representation; or
- `Terminal(audit_sha256)` for exactly `B1 []`.

`audit_sha256` content-addresses the verified preprocessing report that ended at
the vertex. Snapshot metadata supplies the single model ID, objective, adapter
and controller-start rule against which all rows were generated. The snapshot
writer rejects missing attestations, non-CC preferences, illegal CC coordinates
and false terminal claims.

A database hit means that at least one successfully preprocessed exact key is
present. A miss says only that current graph coverage did not contain the
generated stopping point.

## Checkpoint replacement

A better neural checkpoint never mutates the meaning of an existing snapshot.
Upgrade is a new, atomic snapshot generation:

1. copy the new inference checkpoint to immutable storage and assign its
   SHA-256 model ID;
2. query the new clean L1000 policy directly on every old normalized production
   vertex, without rerunning the initial reducer;
3. classify a vertex as retained only when the new model already prefers a
   legal CC there (or it is `B1 []`);
4. for every stale vertex, replay the new model's bounded zero-CC route with
   normalization-only transitions to a new stopping point and retain that route
   as migration evidence;
5. append a stale target's zero-CC migration to incoming proof macros and
   renormalize their endpoints; archive, rather than silently reinterpret,
   active edges whose source vertex disappears;
6. deduplicate new endpoints by `rep_key`, recompute materialized unknotting and
   ACS10 routes, and emit one new-model attestation for every surviving vertex;
7. publish only if attestation coverage equals the production node count and no
   required migration ended in a bound, cycle, `PASS` or other incomplete stop.

The old model/snapshot pair remains live while this audit runs. Publication is a
single snapshot swap and rollback selects the old pair. Proof programs remain
mathematical evidence after a model change, but the active stopping-point
compaction and routing overlay are snapshot-specific and may be rebuilt.
