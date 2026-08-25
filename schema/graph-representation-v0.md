# Graph representation and mirror-orbit key v1

Normative for non-synthetic graph snapshots with `schema_version = 0`.

## Geometry and the origin

The representation is a closed braid on a **cylinder**, not an unmarked torus.
Let `x` be the strand coordinate and `y` the cyclic word coordinate.

- `x = 0` is the marked boundary immediately before strand 1. Generator
  `sigma_g` crosses strand positions `g-1` and `g`. Generator labels are not
  shifted during normalization.
- Mirror parity is chosen first by the rule below; `y = 0` is then the seam
  before the lexicographically least cyclic rotation of that word. Letters are
  compared as signed integers, so negative letters precede positive letters.
- If a periodic word has several equal least rotations, choose the smallest
  left-rotation offset.
- The empty word has offset zero.

Thus `(0,0)` is the intersection of the marked first-strand boundary with the
canonical word seam. There is no search for a geometric point on a continuous
torus.

The optional `B*` alphabet adds the explicitly verified BKL seam band `+/-n`,
but does not remove the marked strand boundary. A future fully toric codec may
also minimize over strand relabellings, but it needs a separately verified
transport for stabilization and destabilization and must use a new codec and
normalizer version.

## State

```text
BraidRepresentation {
    strands: u16,                  // at least 1, at most 32767
    cyclic_band_generators: bool,
    word: Vec<i16>                 // no zero padding
}
```

For the ordinary alphabet every letter satisfies
`1 <= abs(letter) <= strands-1`. In `B*`, `abs(letter) = strands` is also
allowed when `strands >= 3` and denotes the BKL band between the last and first
strand positions.

## Byte codec

Name: `unknotdb-braid-cylinder-le-v0`.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII magic `UKB0` |
| 4 | 1 | codec version, zero |
| 5 | 1 | flags: bit 0 is `cyclic_band_generators`; other bits zero |
| 6 | 2 | strand count, little-endian `u16` |
| 8 | 4 | word length, little-endian `u32` |
| 12 | `2*length` | signed letters, little-endian `i16` |

There are no trailing bytes, capacity padding or inactive strands.

## Mirror parity, normalization and key

Normalizer: `mirror-writhe-word-necklace-v1`.

For a word `w`, its mirror is `mirror(w)[i] = -w[i]`. The graph is keyed by the
two-element orbit `{closure(w), mirror(closure(w))}` because unknotting number
and crossing-change cost are mirror invariant. This does **not** assert that a
chiral knot is isotopic to its mirror.

Choose mirror parity deterministically:

1. if writhe `sum(w)` is positive, keep `w`;
2. if it is negative, use `mirror(w)`;
3. if it is zero, origin-normalize both words and choose the lexicographically
   smaller; choose unmirrored on an exact tie;
4. choose the least cyclic rotation of the selected word, breaking periodic
   ties by the smallest rotation offset.

The normalizer applies no RI/RII, braid relation, Markov move, reversal,
inversion, or generator relabelling. The stable key rule is:

```text
rep_key = SHA-256(Encode(MirrorOrbitNormalize(input)))
```

The normalizer returns `(mirrored, rotate_word_left=r)`. Replay first negates
all letters when `mirrored=1`, then applies

```text
normalized[k] = selected[(k+r) mod length].
```

Mirror action transport is exact:

- `INSERT(..., sign)` negates `sign`;
- `STABILIZE_POS` and `STABILIZE_NEG` exchange;
- every other action kind, generator and cyclic position is unchanged.

After that transform, a raw action position `p` moves to normalized coordinates
by

```text
p_normalized = (p - r) mod length.
```

Conversely, a stored normalized action is transported to the submitted input by

```text
p_input = (p_normalized + r) mod length.
```

Global Markov actions carry no position. Generator numbers are unchanged
because the strand origin is fixed. A target is mirror-orbit normalized afresh
after the complete edge program:

```text
target_key = Key(MirrorOrbitNormalize(Replay(program, source))).
```

The separate necessary knot-closure filter
`word_length = strands-1 (mod 2)` follows from permutation parity. It can reject
about half of arbitrary words early, but it is not mirror parity and never
identifies two keys.

## Semantic action codec

Name: `unknotdb-semantic-action-u63-v0`. It is independent of a network's
capacity-dependent flat policy-head index.

| Bits | Field |
|---|---|
| 0..3 | kind: REDUCE=0, COMMUTE=1, BRAID=2, INSERT=3, DESTABILIZE=4, STABILIZE_POS=5, STABILIZE_NEG=6, PASS=7, CROSSING_CHANGE=8, STABILIZE_AT=9 |
| 4..35 | `u32` cyclic word position, or exact word-gap position for STABILIZE_AT |
| 36..51 | `u16` INSERT generator; zero otherwise |
| 52 | INSERT/STABILIZE_AT sign: 0 positive, 1 negative; zero otherwise |
| 53..63 | reserved, zero |

For global actions all payload fields are zero. `STABILIZE_AT(position, sign)`
is the proof-capable Markov stabilization that inserts the new top generator at
an exact gap `0..=word_length`; it is required for a state-aware inverse of a
middle-word `DESTABILIZE`. Policy heads may continue to propose only the global
stabilizations. Action legality and CC cost are
not inferred from these bits: the independent verifier replays the decoded
action on its input checkpoint.

## Proof-program codecs

### Version 0: endpoint-normalized

Name: `unknotdb-semantic-program-le-v0`.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | ASCII magic `UKP0` |
| 4 | 1 | program version, zero |
| 5 | 3 | reserved, zero |
| 8 | 4 | action count, little-endian `u32` |
| 12 | `8*count` | semantic actions as little-endian `u64` |

An edge program is non-empty and cannot contain `PASS`, which is a policy
controller decision rather than a topological primitive. The validator checks
each action before applying it, validates the resulting representation after
every action, recomputes the number of crossing changes, normalizes the final
state and compares the exact encoded target checkpoint.

Production snapshot edges must record validator version
`unknotdb-runtime-braid-validator-v1`. The snapshot writer runs that validator;
the string alone is not accepted as evidence. Version 0 remains readable for a
legacy macro whose action coordinates are continuous and which needs only the
implicit final normalization.

### Version 1: checkpointed origin

Name: `unknotdb-semantic-checkpoint-program-le-v1`. It uses the same `UKP0`
header and instruction count, with header version byte one. Every instruction is
one little-endian `u64`:

- bit 63 zero: the complete value is a v0 semantic-action `u63`;
- bit 63 one: bits 61..62 select an orbit/checkpoint instruction, bits 0..31
  contain its `u32` payload and bits 32..60 must be zero:
  - opcode 0: `NORMALIZE_ORIGIN(rotate_word_left)`;
  - opcode 1: `ROTATE_ORIGIN_LEFT(amount)`;
  - opcode 2: `MIRROR_ORBIT`, whose payload must be zero;
  - opcode 3 is reserved.

`NORMALIZE_ORIGIN(r)` is a zero-cost coordinate instruction, not a knot move.
The independent validator recomputes the canonical origin of the current raw
representation, requires its shift to equal `r`, and only then applies the
rotation. This makes an alternating policy route replayable without storing its
intermediate representations.

`ROTATE_ORIGIN_LEFT` is the explicitly replayed inverse coordinate operation.
`MIRROR_ORBIT` changes the current representative to its mirror. It is a
cost-preserving symmetry step in the quotient graph, not an isotopy. Therefore
an orbit edge proves the Bellman inequality

```text
U(orbit(source)) <= cc(program) + U(orbit(target)).
```

To emit a concrete path for one chirality, the executor carries one mirror bit
and mirrors the remaining semantic program whenever it crosses a
`MIRROR_ORBIT` boundary. It must never print `MIRROR_ORBIT` as a Reidemeister or
Markov move.

A v1 weighted graph macro:

- starts with a semantic action, never an origin instruction;
- contains zero or one crossing changes, with the exact count stored as
  `cc_cost`;
- contains exactly one crossing change when it is the canonical policy edge
  from one nonterminal stopping point to the next;
- may contain zero crossing changes when it records a verified equivalence or
  a reverse-scramble route between two knowledge states;
- may then contain zero-cost semantic moves and origin normalizations;
- ends at the exact normalized target encoding, with no implicit missing
  checkpoint;
- has a database `program_version` equal to its header version.

This distinction is necessary because normalization can occur between policy
decisions. Merely normalizing the final result does not in general preserve the
coordinates of a later stabilization, insertion or other semantic action.
