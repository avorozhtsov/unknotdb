# ASC10-first catalogue discrepancy: 8_9

## Selection

- Parent graph: `outputs/unknotdb-rf-spherogram-expanded-v0.sqlite`
  (`53be2951728a0996ce094f28fc8b34397a04dd098910e27f5fee96ef27644257`).
- Catalogue snapshot: KnotInfo XLS retrieved 2026-08-14,
  SHA-256 `1829b7056eefc653f77a42acc9e471df4020e64fe93a8b6a211ca3d49fe86e7b`.
- Deterministic order: `(ASC10, node_id, knot_id)` over nodes for which the
  catalogue upper endpoint is below replay-validated graph `U_upper`.
- First row: node `62796`, attested identity `8_9`, source representation
  `braid:ea550c2962a09875437cdaf8d7067d91df2ef12378a65d0bfad17fda3d498f51`,
  `ASC10=48`, graph `U_upper=2`, KnotInfo `U=1`.

The catalogue value is also independently visible in the Knot Atlas entry for
`8_9`: https://www.katlas.org/wiki/8_9 . The value alone was treated as metadata,
not as a graph edge.

## Replayable witness

The canonical source vertex is the ordinary three-strand Artin braid

`[-1,-1,-1,2,-1,2,2,2]`.

The accepted unanchored semantic program is:

1. `CrossingChange(position=4)`;
2. `Insert(position=6,generator=1,sign=-1)`;
3. `Insert(position=9,generator=1,sign=+1)`;
4. `Braid(position=7)`.

It contains exactly one CC. The zero-cost moves reach existing node `35818`,
whose canonical representation is
`[-1,-1,-1,2,1,2,-1,2,1,2,-1,2]`. Existing zero-CC edge `119827` then reaches
the canonical unknot node `21019`. The anchored stored edge is `153592`, program
`17228`, with `(anchor_x,anchor_y)=(1,4)`; materialization and replay reproduce
the exact target key.

The normalized source braid independently recomputes determinant `25`, so it is
not the unknot. Together with the one-CC route this happens to prove exact
`u=1`, although catalogue claims remain a separate metadata layer.

## Result and validation

- Published graph: `outputs/unknotdb-8_9-u1-v0.sqlite`
  (`132b66d01ec990dfec9e088acf8639cedcf1e403ba9298da4a1adc83ce68d600`).
- Node `62796`: `U_upper 2 -> 1`, `ASC10 48 -> 43`.
- Growth: nodes `100622 -> 100622`; edges `153592 -> 153593`; programs
  `17228 -> 17229`.
- Four collateral `U_upper` improvements were retained by graph relaxation.
- Search budget: depth `6`, states `200000`, simulations `200000`; used
  `200000` states and `74050` candidate evaluations, with 29 exact graph hits.
- Full publication replay/hash validation accepted all `153593` edges.
- SQLite `integrity_check`: `ok`.
- B4 regression: `286334/286334` seeds covered; all `153593` edges replayed.
- Runtime tests: 63 passed (62 unit tests plus one RF reference-vector test).
- Runtime formatting and strict Clippy passed; catalogue importer Ruff and
  bytecode checks passed.

The refreshed catalogue queue is
`outputs/unknotdb-catalogue-u-asc10-queue-v1.tsv`. After removing this resolved
case (and one collateral resolution), its first row is now `10_141`, node
`47951`, `ASC10=50`, graph `U_upper=2`, catalogue `U=1`.
