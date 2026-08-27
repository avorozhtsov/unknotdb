# Unknot DB roadmap stages 1–5: measured result

## Final immutable graph

- Snapshot: `outputs/unknotdb-rf-spherogram-expanded-v0.sqlite`
- SHA-256: `53be2951728a0996ce094f28fc8b34397a04dd098910e27f5fee96ef27644257`
- Policy: frozen `q-grown-raster-axial-12:Q254:36d1122f5494d502e556994083a1a69adf1643d5be95cd9f80ffc13b68e68d63`, L1000
- Adapter: `initial-reducer-clean-controller-acyclic-top1-self-loop-skip4-mirror-orbit-v4`
- 100,622 nodes; 153,592 immutable replay-validated edges; 17,228 anchored program templates
- 27,557,888 bytes (26.28 MiB)
- `U_upper` p50/p95/max = 4/11/31; 5,204 nodes have `U_upper>10`

Relative to the schema-v3 B4/RF baseline, the complete five-stage run added 5,980 nodes, 6,652 edges and 1,960 exact program templates for 1,605,632 bytes (1.53 MiB).

## 1. Fixed RF high-U cohort

- Durable cohort: 978 RF representations with baseline `U_upper>10`.
- Frozen-policy bounded run: 1 strict improvement.
- Exact one-CC graph branch: 2 further strict improvements.
- Anchored-template run: 19,012,700 bindings evaluated, 589,750 legal, 17,829 exact graph hits, no further strict improvement.
- Exact improvements: `11→10`, `12→10`, `11→10`; aggregate decrease 4.
- Remaining cohort members above 10: 975. This is a truthful negative result: the small bounded search did not make most existing witnesses competitive.

Artifacts: `outputs/unknotdb-rf-gt10-cohort-v0.tsv`, `outputs/unknotdb-rf-gt10-policy-v1.tsv`, `outputs/unknotdb-rf-gt10-branch-v0.tsv`, `outputs/unknotdb-rf-gt10-templates-v0.tsv`.

## 2. Policy adapter v4 and six RF misses

- v4 rejects an internal controller action that would revisit an already seen controller state and deterministically selects the next legal network action, with a hard skip bound. The checkpoint and logits are unchanged.
- A separate v4 snapshot conservatively re-attested all 94,648 existing nodes. This migration is sound because v3 graph nodes were admitted only after terminal or preferred-CC outcomes; the new fallback branch occurs only after a cycle and therefore cannot change those completed v3 outcomes.
- The four controller-cycle misses are now connected: one by graph-assisted bounded policy search (`U_upper≤6`) and three by exact descending certificates (`U_upper≤7,16,11`).
- Two DKT words remain outside Q254's fixed `max_len=48` input capacity (both have length 53). A bounded commute/braid search explored over 300,000 normalized states through depth five without finding a shorter representative; they were not inserted under a false policy-stop claim.

Artifacts: `outputs/unknotdb-rf-policy-adapter-v4-v0.tsv`, `outputs/unknotdb-rf-policy-v4-controller-v0.tsv`, `outputs/unknotdb-rf-policy-v4-descending-v0.tsv`.

## 3. Semantic program-template layer

- 17,228 exact anchored templates grouped into 1,046 coordinate-independent semantic opcode families.
- 952 weighted contiguous semantic subsequences of length 1–4.
- 6,216 independently verified reverse-endpoint edge pairs.
- Utility table records edge use, selected U/ACS10 route use and Bellman slack per exact template.
- Reverse-endpoint pairing is deliberately weaker than claiming syntactic inverse programs; both directions remain independently replay validated.

Artifacts: `outputs/unknotdb-template-analysis-v4-v1.md` and its `-families.tsv`, `-subsequences.tsv`, `-reverse-pairs.tsv`, and `-utility.tsv` companions.

## 4. Knot identification and invariants

- Separate metadata-only SQLite sidecar: `outputs/unknotdb-identification-v4-v1.sqlite`.
- SHA-256: `4882c9e8eb120c67fc2e2aca1c1552b4b11491c99dbbab331357e694d36fb45b`.
- 3,519 unique source representations, of which 3,476 have attested stopping keys in the final graph.
- 3,415 representations have provenance-bearing canonical names; 2,870 bundled named knots have determinant, exact recomputed Alexander polynomial and bundled Jones polynomial.
- Signature is explicitly absent because Spherogram's signature backend is unavailable in the RF environment used for this pass.
- Names, invariants and tabular U claims do not alter proof-graph `U_upper` and are not presented as certificates or lower-bound proofs.

Artifact: `outputs/unknotdb-identification-v4-v1.md`.

## 5. RF exclusion expansion

- Starting exclusions: 540 source records.
- Spherogram generated 531 unique one-component braid representations.
- 490 unique representations fit Q254's `strands≤12`, `word_length≤48` capacity; 489 were newly imported and one already hit the graph.
- Growth from this publication step: 5,937 nodes and 6,606 proof edges.
- 41 generated representatives remain outside Q254 capacity.
- Four source records remain unresolved: two intentional multi-component test links and names `10_1`, `10_3`, which the installed Spherogram catalogue did not resolve under the supplied identifiers.

Artifacts: `outputs/rf-knots-spherogram-expansion-v0.json`, `outputs/rf-knots-spherogram-expansion-v0.md`, `outputs/unknotdb-rf-spherogram-expanded-v0.tsv`.

## Validation

- Full publication replay/hash validation: 153,592/153,592 edges.
- SQLite integrity: `ok` for graph and identification sidecar.
- Protected B4 coverage: 286,334/286,334 (`outputs/unknotdb-final-b4-regression-v0.tsv`).
- Rust: 62 unit tests plus RF reference-vector test passed.
- Rustfmt passed; Clippy passed with warnings denied.
- Ruff passed for all new/changed Python tools.
- No policy training or checkpoint mutation; no RF Knots or pgx-mcts-bench writes; no commit, push or deployment.

## Honest remaining work

The main quality deficit is still the 975 original RF high-U cohort members above 10 and the newly added straightforward certificates that raise the global maximum to 31. The next quality pass should use knot-name/invariant buckets and best known RF witnesses to target those routes, rather than expand the census further. The two length-53 DKT inputs need either an exact Markov-equivalence shortening certificate into Q254 capacity or a separately attested larger-capacity policy; silent truncation is not acceptable.
