# Roadmap

## v0 — done

- Diagram model: darts, faces, planarity validation (`F = n + 2`), components,
  orientation, crossing signs, writhe.
- Canonical signed Gauss signature as the primary key.
- Braid closure input (`braid:s:w1,w2,...`), positive generators giving positive
  crossings.
- Moves: `R1+`, `R1-`, `R2+`, `R2-`, `R3`, `XC` — alphabets `R` and `X` are
  complete. Legality checked before application, result re-validated for
  planarity.
- Certificate format, parser, verifier, content-addressed ids.
- Bootstrap search producing traces: greedy reduction, bounded breadth-first
  search over R3 to unlock further reduction, crossing changes on top.
- Certificates for 3_1, 4_1, 5_1, 7_1, 8_19, 9_1, 10_124 with costs
  1, 1, 2, 3, 3, 4, 4 -- every one matching the published value, and the three
  torus knots matching Kronheimer-Mrowka's (p-1)(q-1)/2.
- `[lib]` + `[[bin]]` split, so the crate can be depended on. 36 tests: 31 unit,
  4 integration exercising the public API as an external caller, 1 doctest.
- Canonical keys correct for links, not just knots.
- Cyclic (cylinder) braids: `cbraid:n:...`, generators mod `n`, where `+/-n` is
  the seam band. Differentially tested against rf-knots over 750 random words
  and 1081 seam generators with zero disagreements.
- No dependencies.

## v0.2 — R3 — done

The derivation, kept here because it is the part that is easy to get subtly wrong.

Let the triangle face have darts `d1, d2, d3` in `phi`-order, `phi = sigma . alpha`.
Write `a_i = alpha(d_i)`, so `d_{i+1} = sigma(a_i)` and `x_{i+1} = cr(a_i)`. At
crossing `x_i` the two triangle darts are `a_{i-1}` (position `p`) and `d_i`
(`p+1`); the outer darts are `u_i` (`p+2`) and `v_i` (`p+3`). Strand `E_i` runs
from `v_i` at `x_i`, along the arc `{d_i, a_i}`, to `u_{i+1}` at `x_{i+1}`.

Legality is that the triangle is *layered*: of the three strands, exactly one is
over at both its crossings, one under at both, one mixed. A cyclic pattern is
not an R3 triangle.

The move flips the triangle to the other side. Checking against explicit
coordinates — two lines crossing at `P` with a third passing below, then above —
the triangle corner moves to the **opposite** corner at every crossing, not just
at `P`. Since over/under must be preserved, `E_{i-1}` stays on the slot pair
`{p, p+2}` it already occupies, and the two internal darts must be adjacent.
Those two facts together leave exactly one configuration, so the rewiring is
forced:

- the outer arc that met slot `v_i` now meets slot `a_i`
- the outer arc that met slot `u_{i+1}` now meets slot `d_i`
- the internal arcs become `{u_{i+1}, v_i}`

Slots keep their positions, so the "under at 0 and 2" invariant is untouched.
Planarity is re-validated on the result anyway.

**Ground truth.** The braid relation *is* an R3 move, so applying it inside a
longer word gives two diagrams that differ as diagrams but represent the same
link. `r3_realises_the_braid_relation` checks five such pairs against closures
built independently by `from_braid`. R3 is also checked to be an involution and
to preserve writhe, crossing count and component count.

## v0.3 — R2+ — done

Push a finger of the arc at `d1` across the arc at `d2`; the two darts must lie
on a common face.

The wiring depends on which way the face walk runs, and that is the part worth
recording. `phi = sigma . alpha` keeps the face on the **right** of each
directed arc, so it runs clockwise around an inner face — the mirror of the
naive counterclockwise sketch. The two arcs therefore run antiparallel across
the shared disk and meet the new crossings in opposite orders:

```text
  A:  d1 -> P -> Q -> e1
  B:  d2 -> Q -> P -> e2
```

with counterclockwise rotations

```text
  at P:  [B->Q,  A->d1,  B->e2,  A->Q ]
  at Q:  [B->P,  A->P,   B->d2,  A->e1]
```

`over` shifts where that cyclic order starts, moving `A` between the odd (over)
and even (under) slots.

**How this was pinned down.** Two hand-drawn derivations both failed planarity,
in the same way, which was the signal that the error was not in the P/Q ordering
but in the face-walk orientation. Rather than keep guessing, the eight candidate
wirings (B's crossing order, and the rotation at each new crossing) were scanned
against every legal dart pair on every face of several diagrams, scoring each by
"planar, and reduces back with writhe preserved". Exactly one combination scored
perfectly and it did so on every pair — an unambiguous determination rather than
a plausible story. The scan also caught a bad *test*: comparing the reduced
result against an unreduced input, which made a third of the pairs look broken.

Alphabets `R` and `X` are now complete. The 2.46M hard unknot diagrams from
arXiv:2409.09032 are the natural regression suite.

Note that the bootstrap search does **not** use R2+ — crossing-increasing moves
blow the search space up, and simplification heuristics are a separate problem
from verification. The verifier accepts R2+ traces from any producer;
`a_certificate_using_r2_plus_verifies` covers that path.

## v0.4 — alphabet M (Markov)

**Reprioritised from v0.5 on evidence, not taste.** Running the verifier over
rf-knots' evidence corpus (100 knots with replayable paths) measured how its
semantic move alphabet lands on this one:

| rf-knots move    | count | unknotdb |
|------------------|-------|----------|
| REDUCE           |   408 | `R2-` |
| CROSSING_CHANGE  |   368 | `XC` |
| BRAID            |   116 | `R3` |
| COMMUTE          |    36 | planar isotopy, no move needed |
| INSERT           |    28 | `R2+` |
| DESTABILIZE      |   473 | **Markov** |
| STABILIZE_NEG    |   228 | **Markov** |

Markov moves are 701 of 1657 moves, 42% of everything, and **0 of 100 paths are
ingestible without them**. Corpus work that depends on ingesting real evidence
is therefore blocked on `M`, not the other way round.

The same measurement explains a search failure. Against the 72-knot DKT 2026
benchmark, the bootstrap search reproduced only 1 of 30 published unknotting
numbers, exhausting in milliseconds even on minimal-crossing diagrams at
`--r3 4`. Destabilisation is how a braid closure actually shrinks; without it
the search stalls on 11- to 13-crossing knots. So `M` buys the search as much as
it buys the verifier.

Cross-check results worth keeping (see also the note on differential testing
below):

- 144 external encodings (72 PD + 72 braid words) parsed, all planar and
  1-component; 100/100 evidence start words after the `B_1` fix.
- Zero soundness violations: over 30 knots with published `u`, the search never
  found an unknotting shorter than the literature. This is the check that would
  have caught a bad R3 or R2+ rewiring.
- 98/98 non-trivial states taken immediately after the last crossing change
  reduced to the unknot under this verifier's own R1/R2/R3 — independent
  agreement with rf-knots, no shared code.

## Link support

Three bugs, all found by running the verifier over real corpora and by
randomised differential testing rather than by reading the code. None affected
knots — every certificate in `certs/` still verifies byte-for-byte — but all
three were real.

- **`canon` was wrong for links** — fixed. It walked `2n` steps from one dart,
  which on a multi-component diagram cycles inside a single component and
  repeats instead of covering the diagram. Now one code per component with `|`
  between them, minimised jointly over component orderings because a shared
  crossing takes its id from whichever component is walked first.
- **The key depended on component orientation** — fixed. Reversing one
  component negates every crossing between components, so relabelling silently
  changed the key. Now minimised over all `2^c` orientations, which makes it an
  unoriented-link key. Caught by randomised PD round-trip testing, not by
  inspection.
- **A free circle did not change the key** — fixed. A knot and that knot split
  off from an unknotted circle are different links and were colliding.
- **Split diagrams were rejected** — fixed. Euler's formula holds per connected
  component, so `sigma_1 sigma_3` in `B_4` has `n + 2k` faces, not `n + 2`.

Still open: PD notation cannot express a crossingless circle, so a subject with
free circles cannot be written faithfully in `subject.pd`. The format needs an
explicit component count before it can carry one.

## v0.5 — corpus

- Ingest rf-knots evidence as certificates, once `M` lands.
- Ingest KnotInfo bounds as `cited` / `pending` records (via
  `soehms/database_knotinfo` CSV, recording the source chirality convention).
- Ingest Brittenham's 12- and 13-crossing crossing-change data; re-verify at L2
  what can be re-verified, and say plainly what cannot.
- Quasipositive braid certificates: cheapest to verify, generated directly from
  a braid pipeline, and sharp for slice genus via slice-Bennequin.

## v0.6 — alphabet B (bands)

`slice_genus_le` computes the genus from the trace via
`chi = births + deaths - bands` rather than trusting the author's number.

## Later

### RF Knots 4k coverage gate for policy upgrades

Before publishing a graph snapshot for a new neural checkpoint, rerun a
read-only coverage regression over the roughly 4,000 knots in the RF Knots
project. This is separate from reattesting the vertices already present in an
Unknot DB snapshot.

The planned gate is:

1. freeze a versioned RF Knots corpus manifest containing a stable knot ID and
   one or more of its simplest natural cyclic torus-braid representations;
2. independently normalize and preprocess every selected representation with
   the candidate checkpoint's exact L1000/reducer/adapter contract;
3. perform ordinary exact-key Unknot DB lookup for every successfully produced
   stopping point;
4. require at least one selected representation of every corpus knot to hit the
   candidate snapshot;
5. compare old-model and new-model results, recording input representation,
   old and new stopping keys, zero-CC migration witness, lookup result and
   reason for every loss of coverage;
6. do not publish the new model/snapshot pair while any corpus knot has no hit.

This check is necessary even after complete internal vertex reattestation. A
new policy can advance a formerly terminal preprocessing point through zero-CC
moves, producing a new stopping key that is absent from the compacted graph.
The 4k gate detects that external reachability regression. Its corpus-selection
rule and manifest format must be frozen before the first baseline run so a
model upgrade cannot improve its score merely by changing which input
representations are tested.

- Nightly full re-verification; per-PR verification of changed certificates only.
  At ~1-10 ms per trace, 60k certificates is minutes and 1.7M is hours, so the
  split matters.
- Traces past ~14 crossings exceed what a git repo should hold: content-address
  the blobs, or store `(seed, tool version, input hash)` and regenerate.
- The semantic-move layer used in rf-knots, as an *optional authoring* format
  with a lossless compiler down to `R`/`M`. Verification must never depend on it.
- Keep a second, independent implementation of the action semantics and check
  the two against each other in CI. The 98/98 agreement above is only evidence
  because rf-knots and unknotdb share no code. Collapsing to one implementation
  would delete the evidence along with the duplication.
- Populate the implemented immutable SQLite/in-memory graph runtime with real,
  independently replayed proof macros. The first unknot/trefoil bootstrap and
  monotone `U_upper(source) <- cc(edge) + U_upper(target)` relaxation are now
  implemented. Bounded deterministic frontier scrambles, durable manifests and
  generic state-aware inverse-witness compilation are also implemented and have
  produced the first live frontier snapshot. Validated multi-generation resume
  and independent ACS10 candidate retention/recomputation are now implemented;
  two resumed generations have produced a validated 6-node, 5-edge proof
  graph. The completed `strands <= 3`, `word_length <= 10` census covers all
  37,958 normalized seeds in a 2,303-node, 2,302-edge graph and establishes
  pilot bounds of 16/4 policy steps and 65,536/16,384 depth-three scramble
  states/candidates. Reducer-output grouping reproduces the length-8 semantic
  manifest exactly and reduces its policy runs from 3,008 to 328 per pass.
  Real-graph benchmarks now measure 242k file/mmap SQLite lookups/s, 1.46M
  deserialized SQLite lookups/s and 13.6M hash lookups/s. Deferring ACS10
  refresh to `O(V log V + E)` and deferring its materialization to one batch
  boundary reduced a complete 2,302-edge rebuild from 5.97 seconds to 20.1 ms
  with identical logical output. The middle-word DESTABILIZE inverse now uses
  an exact positional stabilization; the rank-10 retry completed with 110
  inserted nodes and 374 accepted ACS10 improvements. Clean-controller
  preprocessing now batches JAX state creation and uses CPU network
  microbatches of eight. On the length-8 census this reduced grouped
  preprocessing from about 1.962 s to 0.379 s with an identical semantic
  manifest. Frontier candidates now use the same scheduler in bounded groups of
  256; on rank 10 the oracle consumed about 1.864 s of a 21.06 s run. Next:
  the completed `strands <= 4`, `word_length <= 10` stage covers all 286,334
  normalized seeds in a validated 68,645-node, 69,018-edge graph. Storage
  schema v2 deduplicates its 1,656 proof programs, splits stable keys from dense
  hot rows, and packs B4 words at two letters per byte; the same logical graph
  is now 12.43 MiB instead of 24.74 MiB.
  Its bounded acyclic completion tries preferred CC first and uses depths 2-4
  one-CC proof macros only to escape closed greedy components; macro
  preprocessing is batched. This first snapshot is coverage-first: all routes
  replay, but its maximum `U_upper=2007` is intentionally recorded as a poor
  bound requiring graph improvement, not presented as an unknotting estimate.
  The in-memory backend remains in the multi-million lookup/s range. Exact edge
  identity now has an expected-O(1) hash index with bytewise collision checks;
  bulk insertion already defers ACS10 materialization to one batch boundary.
  Next: run a short-witness improvement pass at this scale, then continue
  controlled frontier population. Static site and read-only API come after the
  population loop is reproducible.

## Deliberately not planned

Proof-assistant formalisation. Invariant tables that duplicate KnotInfo.
Certified lower bounds — they do not exist.
