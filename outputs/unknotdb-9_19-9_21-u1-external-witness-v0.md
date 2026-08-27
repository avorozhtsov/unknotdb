# External one-crossing witnesses for 9_19 and 9_21

Status: independently reproduced external diagram witnesses; not yet imported as
Unknot DB proof edges because the post-CC hard unknots still need a replayable
zero-cost braid-move program under the current action codec.

## Environment

- SnapPy 3.3.2
- Spherogram 2.4.1
- Source snapshot: `outputs/unknotdb-12a_1047-u2-v0.sqlite`
- Source snapshot SHA-256:
  `767127197c81117d2ae2e95ffe9b2785b4436d798493f14cc11b0686ff6a3490`

## 9_19

- Current graph node: 74846
- Current graph U upper: 13
- Catalogue U: 1
- Stored canonical braid:
  `B6 [-5,4,3,-2,3,-2,-4,3,5,-4,3,-2,-1,-2,-3,-2,4,1,3]`
- Crossing change: zero-based word position 5, `-2 -> +2`
- Resulting braid:
  `B6 [-5,4,3,-2,3,2,-4,3,5,-4,3,-2,-1,-2,-3,-2,4,1,3]`
- Source DT: `DT[sasdnlgPrImcSofHqeJabk]`
- Result DT: `DT[sasdnlgPrImcSofHqeJaBk]`
- SnapPy simplified fundamental group: one generator, zero relators.
- Spherogram global simplification: 19 crossings to 0, 0 components.

The equivalent standard named Spherogram diagram witness changes crossing index
3 (zero-based), whose PD entry is `(7,2,8,3)`.

## 9_21

- Current graph node: 89211
- Current graph U upper: 10
- Catalogue U: 1
- Stored canonical braid:
  `B6 [-5,4,3,-2,3,-4,3,5,2,3,-1,-2,3,-4,3,-2,1,3,4]`
- Crossing change: zero-based word position 9, `+3 -> -3`
- Resulting braid:
  `B6 [-5,4,3,-2,3,-4,3,5,2,-3,-1,-2,3,-4,3,-2,1,3,4]`
- Source DT: `DT[sasnEhQKOaDLPGBRJfSMcI]`
- Result DT: `DT[sasnEhQKOaDlPGBRJfSMcI]`
- SnapPy simplified fundamental group: one generator, zero relators.
- Spherogram global simplification: 19 crossings to 0, 0 components.

The equivalent standard named Spherogram diagram witness changes crossing index
8 (zero-based), whose PD entry is `(9,17,10,16)`.

## Unknot DB ingestion boundary

Both exact one-letter CCs were tested with the current bounded graph optimizer.
It did not publish a snapshot: after the CC, the mandatory RI/RII reducer makes
no move and frozen Q254 immediately prefers another CC. Although Spherogram
constructively simplifies each result to the empty diagram, it does not emit that
simplification in Unknot DB's semantic braid-action format. Import therefore
requires compiling an explicit zero-CC braid/Markov trace; the graph values remain
13 and 10 until that trace passes the independent validator.
