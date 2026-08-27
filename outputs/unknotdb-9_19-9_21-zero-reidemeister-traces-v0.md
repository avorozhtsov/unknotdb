# Exact zero-CC Reidemeister traces for the 9_19 and 9_21 witnesses

Status: extracted and independently replayed.  These are diagram-level
certificates; they have not yet been published as braid-native Unknot DB proof
edges.

## Contract

- Input is the ordinary Artin closure of the post-crossing-change braid.
- Crossings have Spherogram's stable braid-closure labels `x0`, `x1`, ... .
- Scheduling is deterministic: repeatedly apply the first available RI/RII in
  lexicographic crossing-label order; then apply the lexicographically first
  RIII descriptor; repeat.
- Every operation stores SHA-256 checkpoints of the complete labelled planar
  adjacency state before and after the move.
- Replay reconstructs the braid closure and applies only the recorded local
  RI/RII/RIII operations.  It never calls `Link.simplify`.
- Engine attestation: SnapPy 3.3.2, Spherogram 2.4.1, trace format
  `unknotdb-labelled-reidemeister-trace-v0`.

## 9_19

External one-CC witness, zero-based braid-word position 5:

```text
[-5,4,3,-2,3,-2,-4,3,5,-4,3,-2,-1,-2,-3,-2,4,1,3]
                         -2 -> +2
[-5,4,3,-2,3, 2,-4,3,5,-4,3,-2,-1,-2,-3,-2,4,1,3]
```

Zero-CC trace (13 moves: RI=3, RII=8, RIII=2):

```text
RII(x0,x8) -> RII(x1,x6) -> RII(x12,x17) -> RII(x16,x9) ->
RII(x10,x14) -> RIII(x3:1,x5:0,x11:1) -> RII(x11,x4) ->
RIII(x5:1,x3:0,x13:3) -> RII(x13,x2) -> RI(x3) ->
RII(x15,x5) -> RI(x18) -> RI(x7) -> empty unknot diagram
```

Artifact: `outputs/unknotdb-9_19-zero-reidemeister-trace-v0.json`

Artifact SHA-256:
`6ddb53a2367d9b5c7c6f092f83918fa4508896c39950122b0e16ad2c4f0b31c6`

## 9_21

External one-CC witness, zero-based braid-word position 9:

```text
[-5,4,3,-2,3,-4,3,5,2, 3,-1,-2,3,-4,3,-2,1,3,4]
                                  +3 -> -3
[-5,4,3,-2,3,-4,3,5,2,-3,-1,-2,3,-4,3,-2,1,3,4]
```

Zero-CC trace (13 moves: RI=3, RII=8, RIII=2):

```text
RII(x0,x7) -> RII(x1,x5) -> RII(x10,x16) -> RII(x13,x18) ->
RII(x3,x8) -> RIII(x11:3,x9:2,x12:3) -> RII(x11,x6) ->
RIII(x15:1,x12:0,x9:1) -> RII(x15,x4) -> RII(x2,x9) ->
RI(x12) -> RI(x14) -> RI(x17) -> empty unknot diagram
```

Artifact: `outputs/unknotdb-9_21-zero-reidemeister-trace-v0.json`

Artifact SHA-256:
`4112893a39db217b1b89636659800326b0dc4934f72df43dde33a3ec1bb703f0`

## Validation performed

- Fresh-process replay accepted both traces.
- Two independent extractions of the 9_19 trace were byte-identical.
- A deliberately changed checkpoint was rejected at the first mismatching
  operation.
- Python byte-code compilation of `tools/reidemeister_trace.py` succeeded.

The remaining integration boundary is explicit: the production graph's proof
program codec currently represents braid/Markov primitives, not labelled
planar Reidemeister instructions.  Therefore these witnesses prove the two
post-CC diagrams are unknots, but must not be inserted by pretending that the
13 planar moves are already braid-native programs.  The next safe change is a
versioned planar-certificate instruction/sidecar whose hash is bound to the
edge and whose Rust verifier replays this same local-move contract.
