# Graph runtime v1

Status: implemented storage/lookup substrate plus a bounded adapter for the
existing frozen `q-grown-raster-axial-12` Q254 policy. No network was trained by
this project. A first two-node production bootstrap snapshot now contains the
unknot and one independently replayed trefoil edge; the large benchmark corpus
remains synthetic.

## Trust boundary

The dependency-free diagram verifier remains the authority for its R/X
certificate alphabet. The graph runtime now also contains a separate,
list-based Rust validator for the production cyclic-braid alphabet. The
production snapshot writer replays every primitive, recomputes crossing-change
cost and normalization, and reproduces the target checkpoint before accepting
an edge. It also checks relational invariants and program hashes; a claimed
validator-version string is never sufficient.

`runtime/` is therefore a separate crate. Its dependencies and generated SQLite
files do not enter the verifier's trusted core.

## Representation and key contract

The production key contract is frozen in
[`schema/graph-representation-v0.md`](../schema/graph-representation-v0.md).
The active geometry is a cylinder: the word direction is cyclic and chooses its
origin by least cyclic rotation, while the strand direction retains its marked
first boundary. The graph key additionally chooses one representative of the
mirror pair by writhe sign and a zero-writhe lexicographic tie-break. Its witness
contains a mirror bit and an origin shift, and actions are transported exactly.
This quotients the value graph by the proved symmetry
`U(K)=U(mirror(K))`; it does not call chiral knots isotopic.

For a policy edge, the required endpoint remains

```text
target_key = Key(Normalize(Apply(program, source_representation)))
```

The raw result of `Apply` is an input checkpoint to normalization, not a graph
node.

Production policy macros use checkpointed proof-program v1. Its compact
`NORMALIZE_ORIGIN` and `MIRROR_ORBIT` instructions record intermediate chart
changes and are independently recomputed during replay. Thus a stored edge can
be exactly `CC`, followed by alternating zero-CC moves and normalizations,
without storing intermediate representations as graph vertices. Concrete
certificate export carries a mirror bit and mirrors the remaining action suffix;
it never reports mirror symmetry as a topological move.

## Mandatory preprocessing and policy adapter

The deterministic v0 reducer and the policy adapter are implemented in
`runtime/src/reducer.rs` and `runtime/src/policy.rs`, and specified in
[`schema/preprocessing-v0.md`](../schema/preprocessing-v0.md). Starting from a
normalized origin, the reducer repeatedly chooses `DESTABILIZE` when legal,
otherwise the first cyclic `REDUCE` position. Every primitive strictly decreases
`L10 = 10 * strands + word_length`; the emitted zero-CC program and both
normalization checkpoints are replayed before the result is returned.

After the one initial reduction, policy inference starts from a clean controller
with objective L1000 and follows its top-1 internal controller actions. A
preferred zero-CC semantic action is applied by Rust and only mirror/origin
normalization follows; the reducer is not run again. This preserves a
deliberately expanded state prepared by the policy for its next CC. A
nonterminal representation is admitted only when the next clean semantic
preference is a legal CC, before that CC is applied. Adapter v3 skips at most
four zero-CC proposals that normalize to an already visited key, asking for the
next-ranked action each time; rejected self-loops are not part of the witness.

`R3` and inverse/expanding moves remain outside the decreasing reducer, though a
bounded policy route may propose a permitted zero-CC move that Rust can replay.
The runtime commands `reduce-braid` and `preprocess-braid` expose the two layers
separately.

## Stopping points

A production node is retained only when it is a policy stopping point for the
snapshot's one model/L1000/clean-controller contract. Concretely, its first
semantic preference is a legal CC, or it is the canonical terminal `B1 []`.
Route endpoints, seed nodes and auxiliary L10/L1000 nodes do not override this
rule.

RI/RII intermediate states inside a macro are stored only in its compressed
program. Normalization-only intermediate encodings are never nodes. Snapshot
generation deduplicates every retained node by `rep_key`.

Role bits are reserved for `CORE`, `L10_AUX` and `L1000_AUX`. L10/L1000 routes
will be an overlay over the same representation table, but auxiliary nodes must
still satisfy the same snapshot-wide stopping predicate. The current hot row
does not yet store L10/L1000 values or pointers.

## Reverse population and Bellman relaxation

Population starts from the canonical unknot with `U_upper = 0`. A generator
produces a bounded scramble of a known representation, runs the mandatory
initial-reducer/policy/normalization pipeline, and keeps only an admitted normalized stopping
point. It must also supply a checkpointed macro from that source stopping point
to an already known target. The independent validator replays the macro before
the candidate reaches the graph.

For a verified macro `e: source -> target`, relaxation is

```text
proposed_U(source) = cc(e) + U_upper(target),   cc(e) in {0,1}.
```

The source is inserted when absent and its active unknot route is replaced only
when `proposed_U` is strictly smaller. A verified edge that does not improve U
is still retained when it is an admissible ACS10 candidate; exact duplicate
programs are not retained. Previously accepted edges remain as proof evidence;
only the materialized first-step pointers change. A
snapshot-local `unknot_route_rank` must strictly decrease along the selected
route, so a zero-CC equivalence can never introduce a routing cycle.

The ACS10 overlay is recomputed independently over all retained proof edges at
snapshot publication. An edge is eligible exactly when
`ACS10(target) < ACS10(source) + cc(edge)` and its target is earlier in the
fixed `(ACS10, rep_key)` order. The latter supplies an acyclic rank even for an
equal-ACS10 one-CC step. Among eligible candidates the builder minimizes
`(ACS10(target)+cc, ACS10(target), target_rank, target_key, edge_id)`.

The bootstrap command exercises this path with one deterministic checked
scramble from `B1 []` to the trefoil orbit. The implemented bounded frontier
enumerates deterministic depth-limited scrambles, records every exact scramble,
preprocessing audit and state-aware inverse edge in a durable TSV manifest, and
relaxes accepted vertices into a fresh immutable snapshot. Its first live run
examined two distinct depth-three one-CC candidates: one reduced back to the
unknot and one inserted the trefoil orbit.

`resume-frontier` validates the complete immutable parent, requires the exact
same frozen policy model ID, restores the mutable reverse-DP graph, and expands
one or more route-rank generations. Every generation and seed is recorded even
when it has no candidates; state/candidate truncation is explicit. The first
live resumed Q254 run expanded the trefoil seed through eight candidates and
inserted one new `U_upper=2` stopping point, producing 3 nodes and 2 edges.
The next resumed run expanded that rank-2 seed through all six available
bounded candidates without truncation, inserted three `U_upper=3` stopping
points, and published a fully validated 6-node, 5-edge snapshot with both route
overlays recomputed.

The first exhaustive natural-input census covers `B1 []` and every ordinary
two-strand word of length at most 10 whose closure is a knot. There are 683 raw
inputs and 48 mirror/origin-normalized seeds. They preprocess to five stopping
points; Q254 immediately prefers either terminal or CC after the initial
reducer, with no policy zero-CC moves. Full depth-three scramble enumeration
over those five points observed at most 2,966 states and 561 distinct
candidates. The B2 pilot limits are therefore 16 policy plies, 4 semantic
moves, 4,096 scramble states and 1,024 candidates. Geometry bounds 5 strands
and word length 15 permit all depth-three stabilizations/insertions from the
observed endpoints.

The 6-node frontier snapshot initially missed `B2 [1^7]` and `B2 [1^9]`.
`complete-natural-b2` added replayed macros `T(2,7) -> T(2,5)` and
`T(2,9) -> T(2,7)`, giving `U_upper=3` and `4`. The resulting 8-node, 7-edge
snapshot passes all production validation and a repeated census finds all
48/48 normalized seeds.

The staged three-strand census next covered word lengths at most 6 and 8. At
length 8, 46,547 raw knot-closing inputs collapse to 3,008 normalized seeds and
324 Q254 stopping points. Full depth-three enumeration over every stopping
point observed maxima of 19,171 states and 5,255 candidates without
truncation. The corresponding pilot limits are 16 policy plies, 4 semantic
moves, 32,768 states, 8,192 candidates, 6 strands and word length 14. Completion
inserted 277 independently replayed one-CC macros. The resulting 325-node,
324-edge snapshot finds all 3,008/3,008 normalized seeds.

The reducer-output grouping optimization preserves each input's own reducer
witness, reuses only the deterministic Q254 policy suffix, and independently
verifies every assembled preprocessing report. On the full length-8 census it
reduced 3,008 policy pipelines to 328 per pass while reproducing the old
semantic manifest exactly.

The completed length-10 stage contains 745,427 raw inputs, 37,958 normalized
seeds, 2,313 reducer outputs and 2,303 stopping points. Exhaustive depth-three
measurement observed at most 32,517 states and 9,684 candidates without
truncation, establishing pilot limits 16/4 for policy, 65,536/16,384 for
scrambles, 6 strands and word length 16. Completion inserted 1,978 independently
replayed one-CC macros. The resulting 909,312-byte snapshot has 2,303 nodes and
2,302 edges; an independent repeated census finds all 37,958/37,958 seeds.

The four-strand length-10 stage enumerates 5,182,723 raw knot-closing inputs.
Mirror/origin normalization leaves 286,334 natural seeds; reducer-output
grouping needs 51,576 policy pipelines, which produce 51,515 distinct stopping
points. Of those, 49,212 were absent from the rank-10 parent snapshot. Greedy
preferred-CC completion exposed genuine stopping-graph cycles, so completion
now uses deterministic bounded acyclic search: preferred CC first, then other
CC positions, then replayed proof macros of exact depths 2, 3 and 4 containing
exactly one CC and otherwise zero-cost moves. Macro candidates are preprocessed
in batches, and every selected program is replayed before Bellman insertion.
The published snapshot covers all 286,334/286,334 seeds, inserts 66,232 edges
including intermediate stopping points, and contains 68,645 nodes and 69,018
edges. Its file size is 25,939,968 bytes (24.74 MiB); validated loading takes
0.332 seconds and SQLite `integrity_check` returns `ok`. This is a
coverage-first bootstrap, not an optimized unknotting table: its replayed
routes are valid upper bounds, but greedy/acyclic completion can attach a long
tail (the snapshot-wide maximum `U_upper` is 2,007). A separate relaxation pass
must replace those edges with shorter witnesses; the failed experimental
five-CC rebuild was not published.

Schema v1 compacts that same logical B4 graph to 16,826,368 bytes (16.05 MiB),
a 35.1% reduction. An exhaustive SQL comparison reports zero differences in
nodes, representations, policy stops, and decoded edge semantics. The 69,018
edges reference 1,656 distinct proof programs containing 83,424 program bytes.

Schema v2 separates the stable key map from the dense hot-node arena and packs
the cold braid word. The same graph is 13,029,376 bytes (12.43 MiB), 49.8%
smaller than schema v0 and 22.6% smaller than schema v1. B4 representations use
a four-byte variable-length header followed by two signed generator codes per
byte; their average stored size falls from 31.01 to 9.23 bytes. Larger braid
alphabets automatically use one- or two-byte codes. Canonical keys and proof
checkpoints still use the frozen v0 encoding, so storage compaction cannot
change graph identity.

## SQLite layout

The generated file is an immutable snapshot, not a mutable serving database.

- `meta`: schema, codecs, the snapshot-wide validator version and the single
  policy model ID, L1000 objective, adapter version and clean-controller rule.
- `node_keys`: the 32-byte `rep_key -> node_id` lookup B-tree.
- `nodes`: dense rowid-keyed hot routes; it contains the two materialized
  first-route answers and independent acyclic route ranks for the unknot and
  ACS10 overlays. Splitting the key removes the redundant `node_id UNIQUE`
  B-tree while preserving ordered array loading.
- `representations`: packed normalized storage encodings, separated from the
  hot lookup tables.
- `policy_stops`: one cold attestation per production node, containing the
  preferred CC (or terminal marker) and a content hash of its audit report.
- `programs`: a content-deduplicated cold dictionary. Each row stores the
  program codec version, compressed bytes and one binary 32-byte SHA-256.
- `edges`: source/target headers, a snapshot-local `program_id`, and optional
  certificate provenance. Programs, hashes and validator strings are not
  repeated per edge.

Node and edge IDs are dense unsigned 32-bit integers local to one snapshot. This
permits direct array indexing and means IDs must never be persisted across
snapshot generations; `rep_key` is the stable identity.

The writer consumes node and edge iterators without materializing the whole
graph; only the distinct-program dictionary is retained during the build. It
builds a temporary file, validates it, vacuums it, and publishes it with one
rename. An interrupted build leaves the previous snapshot untouched. Runtime
schema v2 remains able to read schemas v0 and v1, and
`compact-snapshot INPUT OUTPUT` rewrites a validated older artifact without
policy inference.

The mutable population builder maintains a hash index from exact edge identity
to candidate edge IDs. Hash collisions are resolved by bytewise program
comparison, so duplicate detection is expected O(1) instead of scanning all
previous edges. Bulk frontier and census insertion continue to defer the ACS10
overlay and recompute it once per batch.

## Serving modes

Three implementations expose the same exact lookup:

```text
rep_key -> {
  node_id, u_upper_bound, acs10,
  next_unknot: {edge, target, cc_cost, first_action}?,
  next_acs10:  {edge, target, cc_cost, first_action}?
}
```

1. read-only SQLite file with mmap enabled;
2. the SQLite image deserialized into memory;
3. `HashMap<[u8; 32], NodeId>` plus dense node/edge arrays.

The hash snapshot keeps proof programs in SQLite's cold blob arena. Building a
new hash snapshot happens off the request path; `LiveGraph` publishes it via an
atomic `Arc` swap, so existing readers finish on the old generation without a
lock.

## Snapshot invariants

Publication fails unless all of the following hold:

- node IDs and edge IDs are dense from zero;
- every node has exactly one representation and all edge endpoints exist;
- every production node has exactly one stopping attestation and synthetic
  nodes have none;
- snapshot metadata pins one nonempty policy model ID, objective L1000, adapter
  version and controller-start rule;
- every nonterminal attestation contains a legal CC in the node representation,
  while the terminal marker is allowed only for `B1 []`;
- each materialized route is either entirely null or entirely present;
- a selected edge starts at the node and its materialized target, cost and first
  action match the edge row;
- `next_unknot` has zero or one crossing changes and
  `U(source) = cc(edge) + U(target)`;
- the snapshot-local `unknot_route_rank` strictly decreases on every selected
  unknot edge, preventing zero-cost cycles;
- `next_acs10` satisfies
  `ACS10(target) < ACS10(source) + cc(edge)`;
- the snapshot-local `acs_route_rank` strictly decreases on every selected
  ACS10 edge, preventing cycles even when ACS10 is equal;
- every edge has `cc_cost` zero or one and references an existing program;
- program IDs are dense, every program is used, and every compressed program
  matches its single stored binary SHA-256 during the full cold-arena pass;
- every submitted edge uses the snapshot-wide non-empty validator version.

Synthetic snapshots run only the structural checks and are marked non-proof.
For a production braid snapshot, the writer additionally decodes and replays
the complete program, rejects illegal actions and `PASS`, recomputes CC cost,
runs every declared intermediate origin normalization, and reproduces the exact
target encoding. The Rust semantics are differentially checked against 256
frozen vectors generated by RF Knots' list reference implementation, covering
every state-changing action kind.

## Policy checkpoint and upgrades

The current inference artifact is
`q-grown-raster-axial-12:Q254:36d1122f5494d502e556994083a1a69adf1643d5be95cd9f80ffc13b68e68d63`.
The repository stores the 1,036,837-byte checkpoint and provenance manifest in
`models/q-grown-raster-axial-12-q254-frozen-20260824-v0/`. It was copied from one
read of the Q254 state and no training was performed.

A model upgrade is built off-line as a new full snapshot. Every old vertex is
reprocessed under the new frozen model. Vertices whose next semantic preference
is no longer CC are stale: their verified zero-CC migration is used to find a
new stopping point, incoming macros are extended to the new endpoint, and
routes are recomputed. If any production vertex lacks a new-model attestation,
or preprocessing ends by a bound/cycle/PASS rather than CC, publication fails.
The old model/snapshot pair serves throughout the audit; an atomic swap prevents
mixed-model reads and provides rollback.

## Initial benchmark

Measured 2026-08-24 on an Apple M2 MacBook Pro with 16 GB RAM, release build,
warm randomized exact lookups and 5% requested misses. Rows contain synthetic
representations and one edge per non-root node.

| Scale/backend | Load | Queries/s | p50 | p95 | p99 |
|---|---:|---:|---:|---:|---:|
| 100k SQLite file/mmap | 0.80 ms | 226k | 3.63 us | 6.13 us | 9.04 us |
| 100k SQLite deserialized | 10.43 ms | 969k | 1.00 us | 1.38 us | 1.79 us |
| 100k hash + arrays | 318 ms | 4.89M | 0.167 us | 0.333 us | 0.417 us |
| 1M SQLite file/mmap | 0.43 ms | 229k | 3.88 us | 5.79 us | 8.67 us |
| 1M SQLite deserialized | 72 ms | 685k | 1.38 us | 1.96 us | 2.46 us |
| 1M hash + arrays | 3.88 s | 3.27M | 0.291 us | 0.417 us | 0.542 us |

The 1M snapshot is 281.42 MiB and its streaming build took 11.49 seconds. These
numbers compare the storage engines, not real program sizes or policy quality.
They support keeping SQLite as the canonical artifact and using the hash layer
only where sub-microsecond lookup materially matters.

Reproduce:

```bash
cargo build --release --manifest-path runtime/Cargo.toml
runtime/target/release/unknotdb-runtime build-synthetic /tmp/unknotdb-1m.sqlite --nodes 1000000
runtime/target/release/unknotdb-runtime inspect /tmp/unknotdb-1m.sqlite
runtime/target/release/unknotdb-runtime bench /tmp/unknotdb-1m.sqlite --queries 500000 --miss-percent 5
```

## Real length-10 graph benchmark

Measured 2026-08-25 on the 2,303-node, 2,302-edge production graph. Median of
three one-million-query runs with 5% requested misses: file/mmap SQLite handled
242k queries/s, deserialized SQLite 1.46M/s, and hash+arrays 13.6M/s. Median p99
latencies were 6.42 us, 1.08 us and 0.084 us respectively.

The first population benchmark exposed an eager ACS10-refresh bottleneck.
Changing refresh from `O(VE)` nested scans to `O(V log V + E)` sorting plus one
edge scan reduced a full refresh from 7.76 ms to 0.836 ms. A proof-preserving
batch mode also verifies and Bellman-relaxes every edge separately but
materializes ACS10 pointers once at the batch boundary. The complete rebuild is
now 20.1 ms instead of the original 5.97 seconds, about 297 times faster. Its
completion manifest and every logical graph row are identical to the eager
reference; only the snapshot creation timestamp differs.

Warm profiling of the frozen Q254 adapter attributes about 1.84 ms/request to
`game.from_word` including environment/observation raster construction and
about 1.09 ms/network call to the forward pass. Tensor conversion is only about
0.009 ms. After Bellman batching, preprocessing should therefore batch both
observation construction and network forward while preserving clean controller
state and exact per-input replay.

The grouped adapter now sends independent clean-controller decisions in policy
rounds. JAX state construction is vectorized and JIT-compiled, while Torch
forward uses bounded CPU microbatches of eight; every returned semantic move is
still replayed by Rust per input. On the length-8 census (3,008 normalized
inputs, 328 reducer outputs), grouped preprocessing fell from about 1.962 s to
0.379 s (5.2x). A phase profile measured 0.359 s for policy service versus
1.590 s before batching (4.4x): about 0.194 s JAX state/view construction and
0.124 s network forward. The complete semantic manifest is unchanged apart
from timing fields. The full census remains about 20 s because exhaustive
scramble enumeration dominates it.

Frontier candidates use the same round scheduler in bounded groups of 256. On
the rank-10 generation, the oracle served 6,144 observed requests in about
1.864 s (5,759 exact-cache hits, 61 network forwards) out of 21.06 s total.
Thus policy preprocessing is now about 9% of that run; scramble enumeration,
proof assembly/inversion, independent verification and graph relaxation are the
remaining optimization target.

The oracle now also keeps a bounded exact-response LRU within one frozen-model
process. A response is reused only for the same normalized encoding and a
remaining-ply budget large enough to reach the cached clean-controller
decision. In the first 600 length-8 profiling requests this reduced network
forwards from 617 to 337 with an identical semantic manifest. Cache hits took
about 2-3 us inside Python. End-to-end census improved only modestly because
exhaustive scramble enumeration, not policy, dominates that particular run.

On the 68,645-node B4 length-10 snapshot, a one-million-query run with 10%
requested misses measured 237,668 queries/s for file/mmap SQLite, 1,070,512/s
for deserialized SQLite, and 11,007,760/s for hash+arrays. Corresponding p99
latencies were 7.291 us, 1.667 us and 0.208 us. The in-memory representation
reported 79,504 KiB process RSS after load.
