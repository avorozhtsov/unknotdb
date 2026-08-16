# Certificate format

A certificate is a UTF-8 text file: YAML frontmatter between `---` fences,
followed by the move trace. One file, one claim.

```
---
claim: unknotting_number_le
value: 1
subject:
  canon: "O1+U2+O3+U1+O2+U3+"
  pd: "PD[X[1,4,2,5], X[3,6,4,1], X[5,2,6,3]]"
  knotinfo: "3_1"
alphabet: X
chirality: unknotdb
assumes: []
claim_source: "Rolfsen 1976"
witness_author: "unknotdb/0.1.0 reduce"
witness_date: 2026-08-15
---
XC  c=0
R1- c=1
R2- c=0,2
```

## Frontmatter

| field | required | notes |
|---|---|---|
| `claim` | yes | one of [`claim-types.md`](claim-types.md) |
| `value` | for `*_le` claims | the bound being asserted |
| `subject` | yes | `canon` is authoritative; `pd` is the starting diagram; others are cross-references |
| `target` | for `equivalent`, `gordian_distance_le`, `braid_equivalent` | same shape as `subject` |
| `alphabet` | yes | smallest alphabet that suffices |
| `chirality` | yes | `unknotdb` — see [`conventions.md`](conventions.md) §2 |
| `assumes` | yes | list, `[]` if unconditional. Absence is an error |
| `claim_source` | yes | who first asserted the bound |
| `witness_author` | yes | who produced this move sequence |
| `witness_date` | yes | ISO date |
| `supersedes` | no | sha256 of certificates this replaces |

## Trace

One move per line, `#` to end of line is a comment, blank lines ignored.
Crossings and arcs are referred to **by index in the current diagram**, which the
verifier renumbers deterministically after every move — so a trace is only
meaningful when replayed from the top. This is what keeps traces at ~10 bytes per
move instead of ~200 bytes per intermediate diagram.

```
R1+ d=<dart> loop=<0..3>           add a kink on the arc at dart d; `loop`
                                   names the adjacent dart pair of the new
                                   crossing that forms the loop, which fixes
                                   both the side and the over/under of the kink
R1- c=<crossing>                   remove a kink; c must be a monogon
R2+ d1=<dart> d2=<dart> over=1|2   push a finger of the arc at d1 across the
                                   arc at d2; over=1 puts d1's strand over,
                                   over=2 under. d1 and d2 must lie on a common
                                   face and on different arcs
R2- c1=<crossing> c2=<crossing>    remove a bigon
R3  c1=<c> c2=<c> c3=<c>           triangle move; the triangle must be layered
XC  c=<crossing>                   crossing change
```

Alphabet `M` (`CONJ`, `STAB+`, `STAB-`, `DESTAB`) and alphabet `B` (`BAND`,
`BIRTH`, `DEATH`) are specified but **not implemented in v0**. The
verifier rejects them with `unimplemented alphabet` rather than accepting them —
a certificate is never silently trusted for a move the verifier cannot check.

## Verification

```
unknotdb verify <file>
```

1. Parse frontmatter; reject unknown claim types and missing `assumes`.
2. Load `subject.pd`; validate arc pairing, planarity (`F = n + 2`), component count.
3. Recompute `subject.canon` and compare. A mismatch is a corrupt record.
4. Replay the trace. Each move is checked for legality *before* application; each
   move must lie in the declared alphabet.
5. Check the endpoint condition for the claim type.
6. Compute the cost from the trace and compare against `value`. A trace cheaper
   than `value` is accepted with a warning (the bound is not tight and a better
   certificate should be filed); a trace more expensive is **rejected**.

Exit `0` on success. Any failure prints the move index, the reason, and the
diagram state at that point.

## Canonical signature

`subject.canon` is the lexicographically minimal signed Gauss code over:

- all `2n` starting darts,
- both traversal directions,
- and, when the claim is mirror-invariant, both mirrors.

Format: for each crossing encountered, `O` or `U`, the crossing's index in order
of first visit, then `+` or `-` for its sign. Computed by `unknotdb canon`.

For `n` up to a few hundred the naive `O(n^2)` minimisation is far below the cost
of replaying the trace, so no cleverness is warranted.

## File naming

`certs/<claim>/<canon-sha256-first-16>.cert`, with a human-readable symlink or
index entry under the KnotInfo name where one exists. Content-addressed names
mean two people generating the same witness produce the same file, and a
duplicate submission is a no-op rather than a merge conflict.
