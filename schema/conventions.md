# Conventions

Normative. Everything in `certs/` and every implementation must follow this file.
These are the choices that are cheap now and catastrophic to change later.

## 1. Diagrams

The canonical on-disk diagram encoding is **PD code**.

```
PD[X[1,4,2,5], X[3,6,4,1], X[5,2,6,3]]
```

For a diagram with `n` crossings the arcs are labelled `1..2n`, each label
appearing exactly twice. `X[a,b,c,d]` lists the four arc-ends incident to a
crossing in **counterclockwise** order, starting from the **incoming
under-strand**.

Internally the verifier uses a dart (half-edge) model:

- dart index `4c + p` for crossing `c`, position `p in 0..3`, counterclockwise
- `p = 0` incoming under, `p = 2` outgoing under, `p in {1,3}` the over-strand
- `sigma` rotates a dart counterclockwise about its crossing: `4c+p -> 4c+((p+1) mod 4)`
- `alpha` is the involution pairing the two ends of an arc
- faces are the orbits of `sigma . alpha`
- following the strand: enter at `d`, leave at `sigma^2(d)`, next is `alpha(sigma^2(d))`

Every diagram is validated on load:

- each arc label occurs exactly twice
- `V - E + F = 2`, i.e. the face count is exactly `n + 2` (planarity)
- the strand permutation has the declared number of components

A diagram failing any of these is rejected before a single move is applied.

## 2. Crossing sign and chirality

**This is the footgun.** KnotInfo, Knotscape and SnapPy do not agree on
handedness. `u` is mirror-invariant so it hides the problem; `sigma`, `tau` and
`s` are not, so anything pairing an upper bound with a lower bound will silently
break.

unknotdb fixes:

> With the incoming under-strand at position 0, the crossing is **positive** if
> the incoming over-strand is at position 3, and **negative** if it is at
> position 1.

Equivalently: rotate the under-strand direction counterclockwise onto the
over-strand direction for a positive crossing. Writhe is the sum of signs.

Every diagram record carries an explicit `chirality: unknotdb` tag. Diagrams
imported from elsewhere are converted at ingest and the source convention is
recorded in provenance. **Never** store a diagram whose handedness came from an
unrecorded convention.

Mirror images are distinct records. `3_1` alone is ambiguous; use `3_1` and
`m3_1` (or `-3_1` in prose). A claim about `u` may cite either; a claim about a
signed invariant must name the mirror.

## 3. Identifiers

Names are not keys. `10_161` and `10_162` were the same knot (Perko), 16-crossing
knots have no accepted names at all, and every table renumbers eventually.

Primary key: **canonical Gauss signature** — the lexicographically minimal signed
Gauss code over all starting points, both traversal directions, and both
mirror-relabellings when the claim is mirror-invariant. Computed by
`unknotdb canon`, stable across implementations, defined in
[`certificate-format.md`](certificate-format.md).

Secondary, non-authoritative cross-references stored alongside:

- `knotinfo:` KnotInfo name, when one exists
- `dt:` canonical Dowker–Thistlethwaite code
- `regina:` Regina knot signature
- `braid:` an Artin word for a braid whose closure is the knot

Cross-references are *labels*. A wrong `knotinfo:` field is a metadata bug; a
wrong canonical signature is a corrupt record.

## 3a. Links

The canonical signature covers links as well as knots: one signed Gauss code per
component, `|` between them, crossings numbered by order of first visit across
the whole traversal. Because a crossing shared between two components takes its
id from whichever component is walked first, the minimisation is joint over
component orderings, not per component.

Two consequences worth stating plainly.

**The key identifies unoriented links.** Crossing signs of a link depend on the
direction chosen for each component — reversing one negates every crossing
between components, while self-crossings are unaffected — and nothing in a PD
code fixes those directions. The key is therefore minimised over all `2^c`
orientations. That is correct for unknotting and unlinking claims, and wrong for
anything orientation-dependent such as linking number, which would have to pin
the orientation in a separate field.

**PD notation cannot express a crossingless circle.** A split unknotted
component survives in the key, appended as `|U`, but it is lost by a PD
round-trip. So a certificate whose subject has free circles cannot currently be
written down faithfully in `subject.pd`; such a subject needs an explicit
component count before the format can carry it.

## 4. No pairwise equivalences

A certificate never relates two arbitrary representations to each other. Each
diagram carries a proof of equivalence to **the canonical representative of its
class**, and equivalence of any two diagrams is composition through the
canonical form.

Storing pairwise relations gives an `n^2` corpus and an unbounded review burden.
This rule is what keeps the corpus linear.

## 5. Conditional results

A claim may depend on an unproven conjecture. The 43 newly-determined
`<= 12`-crossing unknotting numbers in arXiv:2409.09032 are conditional on
additivity of `u` under connected sum — a conjecture the same group's later work
put under strain.

```yaml
assumes: [additivity-of-u]
```

Assumptions are drawn from a controlled vocabulary in `schema/assumptions.yaml`.
A claim with a non-empty `assumes` is never served as unconditional, and if an
assumption is ever refuted, every claim depending on it is mechanically
retractable in one query. Unconditional claims carry `assumes: []` explicitly —
absence of the field is an error, not a default.

## 6. Provenance

Every certificate records where the claim came from and who produced the witness.
These are different things and both are required.

```yaml
claim_source:   arXiv:2409.09032        # who first asserted the bound
witness_author: agent:rf-knots/v0.4.1   # who produced the move sequence
witness_date:   2026-08-15
supersedes:     [<sha256 of prior cert>]
```

Git history supplies the audit trail. A certificate is never edited in place to
weaken a bound: a better bound is a new file that `supersedes` the old one, and
the old one stays.
