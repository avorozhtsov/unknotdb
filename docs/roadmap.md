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
- 24 tests, no dependencies.

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

- Nightly full re-verification; per-PR verification of changed certificates only.
  At ~1-10 ms per trace, 60k certificates is minutes and 1.7M is hours, so the
  split matters.
- Traces past ~14 crossings exceed what a git repo should hold: content-address
  the blobs, or store `(seed, tool version, input hash)` and regenerate.
- The semantic-move layer used in rf-knots, as an *optional authoring* format
  with a lossless compiler down to `R`/`M`. Verification must never depend on it.
- Expose a `[lib]` target. The crate is bin-only today, so nothing can depend on
  it; that blocks every form of reuse.
- Keep a second, independent implementation of the action semantics and check
  the two against each other in CI. The 98/98 agreement above is only evidence
  because rf-knots and unknotdb share no code. Collapsing to one implementation
  would delete the evidence along with the duplication.
- Generated SQLite/DuckDB build artifact; static site and read-only API on top.

## Deliberately not planned

Proof-assistant formalisation. Invariant tables that duplicate KnotInfo.
Certified lower bounds — they do not exist.
