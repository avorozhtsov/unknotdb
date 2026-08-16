# Claim types

All ten are defined here from day one so the schema never needs a migration.
They are **populated** in the order given in [`docs/roadmap.md`](../docs/roadmap.md);
an empty claim type is expected, a schema change is not.

Every claim reduces to the same shape:

> a sequence of moves carrying diagram `A` to diagram `B`, drawn from a declared
> **move alphabet**, with a **cost** and an **endpoint condition**.

That is why ten claim types need one verifier.

## Move alphabets

| id | Moves | Changes isotopy class? |
|---|---|---|
| `R` | `R1+ R1- R2+ R2- R3` | no |
| `X` | `R` + `XC` (crossing change) | yes |
| `M` | Markov: `CONJ`, `STAB+ STAB- DESTAB` | no (braid closure) |
| `B` | `R` + `BAND`, `BIRTH`, `DEATH` | no (traces a cobordism) |

`R` is a subset of `X` and of `B`; a certificate declares the smallest alphabet
that suffices, and the verifier rejects any move outside the declared set. This
matters: an unknotting certificate that silently used a band move would be
worthless.

## The claims

| `claim` | Alphabet | Endpoint | Cost | Certifiable |
|---|---|---|---|---|
| `equivalent` | `R` | stated target diagram | — | yes |
| `unknot` | `R` | 0 crossings | — | yes |
| `unknotting_number_le` | `X` | unknot | count of `XC` | yes |
| `gordian_distance_le` | `X` | stated target knot | count of `XC` | yes |
| `braid_equivalent` | `M` | stated target word | — | yes |
| `braid_index_le` | `M` | any `n`-strand word | strand count | yes |
| `slice_genus_le` | `B` | unlink | genus of the cobordism trace | yes |
| `band_unknotting_le` | `B` | unknot | count of `BAND` | yes |
| `unlinking_number_le` | `X` | unlink | count of `XC` | yes |
| `quasipositive` | `M` | — (word-level predicate) | — | yes |

Notes on the less obvious ones:

**`unknot`** is a special case of `equivalent` but gets its own type because it is
the substrate everything else is built on, and because the interesting inputs are
*hard* unknot diagrams — the ones where heuristic simplification fails. The 2.46M
diagram corpus from arXiv:2409.09032 is the regression suite.

**`slice_genus_le`** verifies a movie: a sequence of diagrams connected by `R`
moves, `BAND` (oriented saddle), `BIRTH` and `DEATH`. Cost is read off the trace
via Euler characteristic, `chi = births + deaths - bands`, with genus derived for
a connected cobordism to the unlink. The verifier checks the arithmetic; it does
not take the author's word for the genus.

**`braid_index_le`** and the presentation-width claims (`arc_index_le`,
`bridge_number_le`, deferred) are the same pattern: *exhibit a presentation, plus
an `R`-proof that it represents the knot*. They compose out of the other two
claim types rather than needing new machinery.

**`quasipositive`** is the odd one — a predicate on a braid word (a product of
conjugates of positive generators) rather than a move sequence. It is also the
cheapest to check and, via slice–Bennequin, upgrades directly to a sharp
`slice_genus` value. Cheap to verify, cheap to generate from a braid pipeline,
disproportionately useful.

## Explicitly not certified

`volume`, polynomial invariants, homology. These are computations, not witnesses;
reproducibility (tool + version + input hash) is the honest standard and existing
tables already serve them well.

Lower bounds of every kind — `tau`, `s`, `sigma`, `nu+` — are `cited`, never
`certified`. See [`conventions.md`](conventions.md) §5.
