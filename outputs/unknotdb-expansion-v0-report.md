# Unknot DB bounded expansion v0

## Verified facts

The immutable B4 schema-v2 baseline is
`unknotdb-natural-b4-length10-q254-complete-schema2-v1.sqlite`, SHA-256
`2ca737f26640973cd0daf4919484472d823efce248ed184cba7e88064df6384b`.
Its frozen completion manifest has SHA-256
`7a9ba50b960ba900095dfc26c8d306b8c14eacbbc88f35f69a11311032d622cc`.
It contains 68,645 nodes and 69,018 edges in 13,029,376 bytes, covers all
286,334 frozen preprocessed B4 seeds, and has U p50/p95/max 7/1184/2007.

The fixed B4 quality frontier examined the 128 highest-priority nodes under
depth 3, 4,096 states and 256 candidates per depth. It enumerated 33,831 exact
candidates, found 1,491 existing stopping-key targets, improved all 128 seed
nodes through 212 accepted improving edges, and reduced aggregate U by 109,317.
U p95/max became 1160/1888; no exact unknotting-number claim is made.

The bounded reverse/bidirectional phase independently compiled and replayed
both directions of every accepted macro. It added 3 stopping points and 41
verified edges; 12 existing nodes improved. Its immutable snapshot SHA-256 is
`d3bb3f49a3a921ffec1f4be76a133e0c80bc5544573edebecf30f8ca84b7ffbe`.

The selective B5 boundary pilot used 8 short B4 boundary seeds, depth 2, 512
states and 16 candidates per seed. It added 82 verified ACS10-admissible edges
and zero new stopping points. The deterministic 32-input B5 cohort produced 22
distinct normalized stopping points, of which 19 were already covered. The
pilot took 3.555 seconds; the SQLite file did not grow because the new rows fit
existing pages. This was a fixed-budget pilot, not a B5 census.

The final snapshot is `unknotdb-selective-b5-boundary-v1.sqlite`, SHA-256
`73283da090c3ff1066abdf7a87933fd66ccc1387fa3094ad5d8ff8e20a02bf2a`.
It contains 68,648 nodes, 69,434 edges and 1,741 deduplicated programs in
13,131,776 bytes. Full validation replayed all 69,434 edges, verified all
program hashes and SQLite integrity, and retained 286,334/286,334 frozen B4
coverage. Final U p50/p95/max is 7/1160/1888; measured storage is 191.291 bytes
per node and 189.126 bytes per edge.

Action-Choice Score 10 is derived exactly as
`10*strands + 5*U + word_length`. Its pointer is recomputed independently from
the unknotting pointer over all verified immutable edges. Eligibility requires
`ACS10(target) < ACS10(source) + cc(edge)` and a strictly decreasing snapshot
rank. Both routes exist for 68,647 nodes; ACS10 selects a different first edge
for 5 nodes.

Runtime validation passed 52 unit tests, the frozen RF semantic-vector test,
formatting, and Clippy with warnings denied. The independent verifier passed
31 unit, 4 API, and 1 documentation test. RF Knots and pgx-mcts-bench were not
modified, and the frozen Q254 policy was not changed or trained.

## Interpretation and proposal

The quality frontier delivered substantially more value than the small B5
boundary pilot. The pilot's 19/22 coverage and zero new stopping points do not
justify an unbounded full B5 census. A later expansion should target the three
measured cohort misses, or use a larger but still fixed and manifest-backed
cohort, and compare new covered stopping points per byte and per second before
requesting approval for broader enumeration.
