# unknotdb

**A replayable proof graph and command-line knowledge base for knot theory.**

UnknotDB stores a graph whose vertices are concrete knot representations and
whose edges carry explicit, machine-checkable move programs. A path to the
unknot with `k` crossing changes is a witness for the upper bound `u(K) <= k`.
The graph also carries useful sidecars: knot identifications, invariants,
representation features, provenance, routing objectives, lower-bound claims,
and training labels. A database miss means only that the current snapshot does
not cover the query; it is not a mathematical lower bound.

The main user-facing product is the `unknotdb` CLI. It is designed to answer
questions such as:

- Which knot does this representation describe, and which representations are
  known for a named knot?
- Which invariants and representation-dependent features are available?
- Is there a replayable unknotting witness, and what upper bound does it prove?
- How can a representation be converted, normalized, or mapped into the proof
  graph?
- Which knots or representations match a collection of invariant, fingerprint,
  or catalogue constraints?

Today the graph is centered on unknotting-number witnesses. The planned
extension is a **Gordian proof graph** whose paths witness upper bounds on the
crossing-change distance between arbitrary knot types.

## Data and research foundations

UnknotDB builds on existing knot tables, computational archives, and published
results. The important upstream sources are linked here so that the provenance
is visible before the implementation details:

**Imported databases and computational data**

- [KnotInfo](https://knotinfo.org/) supplies canonical names, representations,
  invariants, and published unknotting-number intervals. The federated
  catalogue imports a hash-pinned
  [KnotInfo workbook](https://knotinfo.org/knotinfo_data_complete.xls) through
  13 crossings.
- Mark Brittenham's exhaustive crossing-change archives provide the imported
  [12-crossing sliced data](https://www.math.unl.edu/~mbrittenham2/unknottingsearch/database/12_crossing_knots_sliced.zip),
  [13-crossing sliced data](https://www.math.unl.edu/~mbrittenham2/unknottingsearch/database/13_crossing_knots_sliced.zip),
  and [13-crossing SnapPy identifications](https://www.math.unl.edu/~mbrittenham2/unknottingsearch/database/13_crossing_knots.zip).
- The Dranowski--Guo--Kabkov--Tubbenhauer
  [computational repository](https://github.com/dtubbenhauer/unknot) supplies
  corrected, certificate-indexed lower-bound data associated with *Machine
  learning methods and unknotting numbers*.

**Research used as a basis or regression target**

- Applebaum et al.,
  [*The unknotting number, hard unknot diagrams, and reinforcement learning*](https://arxiv.org/abs/2409.09032),
  motivates the hard-diagram and learning setting and supplies published
  unknotting claims used as regression targets.
- Brittenham and Hermiller,
  [*Unknotting number is not additive under connected sum*](https://arxiv.org/abs/2506.24088),
  supplies the connected-sum counterexample and named proof-chain targets.
- Wang and Zhang,
  [*A remark on the counterexample to the unknotting number conjecture*](https://arxiv.org/abs/2507.14265),
  supplies the direct five-crossing construction replayed as a proof cube.

These sources provide data, claims, diagrams, or candidate move sequences; they
do not bypass UnknotDB's trust boundary. Imported claims remain
provenance-bearing catalogue data until an explicit move program has been
independently replayed.

## The three-project system

| Repository | Responsibility |
|---|---|
| [**rf-knots**](https://github.com/avorozhtsov/rf-knots) | The knot environments and neural models. UnknotDB uses a pinned RF Knots policy during bounded preprocessing to reduce representations before graph lookup and insertion. |
| [**pgx-mcts-bench**](https://github.com/avorozhtsov/pgx-mcts-bench) | The MCTS and training machinery used to learn and search with those models. |
| **unknotdb** | The durable evidence layer: representations, proof programs, derived routes, identifiers, invariants, and other sidecars. |

The dependency is intentionally two-way at the data level. RF Knots models help
preprocess representations so equivalent or unnecessarily large states do not
inflate the graph. In return, UnknotDB exports replay-derived supervision for RF
Knots and is intended to supply training data to **RM Nodes**. Learned policies
may propose routes, but only independently replayed programs enter the proof
graph as evidence.

Despite the name, the certificate machinery is not specific to unknotting. The
same verifier covers slice genus, Gordian distance, braid index and unlinking —
see [`schema/claim-types.md`](schema/claim-types.md).

Most unknotting-number bounds in existing tables are assertions plus citations.
KnotInfo may store `[2,3]` in a cell and link to a paper, while the concrete
diagram and transformations remain embedded in prose or an appendix.

unknotdb stores the **witness**: an explicit sequence of moves that a small,
dependency-free program re-checks from scratch. Certificates live in a git repo,
submissions are pull requests, and CI is the referee.

## Quickstart

The current public-facing artifact is the coherent `v0.10.1` pre-release
bundle. It contains 101,213 graph vertices and 154,828 immutable proof edges;
full release validation replayed every edge. Download and verify the bundle as
described under [Coherent graph release bundle](#coherent-graph-release-bundle),
then try four representative queries:

```bash
tools/unknotdb_cli.py knot-show 9_19
tools/unknotdb_cli.py invariants-for-knot 3_1
tools/unknotdb_cli.py find-knots-by-invariant --invariant determinant=3
tools/unknotdb_cli.py neighbors 9_19 --status replay_verified
```

The Rust runtime validates and queries the immutable proof graph:

```bash
cargo build --release --manifest-path runtime/Cargo.toml
runtime/target/release/unknotdb-runtime validate release-v0.10.1/proof.sqlite
runtime/target/release/unknotdb-runtime lookup-braid \
  release-v0.10.1/proof.sqlite 2 1,1,1
```

The smaller certificate verifier remains available independently:

```bash
cargo build --release --manifest-path verifier/Cargo.toml
UNKNOTDB=./verifier/target/release/unknotdb

# re-check the whole corpus from scratch
find certs -name '*.cert' -print0 | xargs -0 "$UNKNOTDB" verify

# inspect a knot, given a PD code or a braid closure
"$UNKNOTDB" info "braid:3:1,2,1,2,1,2,1,2"

# search for an unknotting sequence and emit a certificate
"$UNKNOTDB" mkcert "braid:2:1,1,1" --knotinfo 3_1 --source "Rolfsen 1976" \
    --date 2026-08-15
```

`unknotdb help` lists the certificate commands. To submit, see
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## What is certified

A certificate is a sequence of moves from one diagram to another, in a declared
move alphabet, with a cost. That single shape covers ten claim types — see
[`schema/claim-types.md`](schema/claim-types.md).

| Status | Meaning |
|---|---|
| `certified` | A move sequence is present and the verifier accepts it. |
| `cited` | A published claim with provenance but no witness. Lower bounds live here permanently. |
| `pending` | Ingested bound awaiting a witness. Not served by default. |

Upper bounds and equivalences can be `certified`. **Lower bounds cannot** — there
is no move sequence witnessing `u(K) >= 3`. They carry tool, version and input
hash for reproducibility instead. When a certified upper bound meets a cited
lower bound at the same value, the invariant is determined; that pairing is the
point of the project.

## Non-goals

- Competing with [KnotInfo](https://knotinfo.org/) as an invariant table. unknotdb
  keys on and links to KnotInfo; it does not duplicate it.
- Proof-assistant formalisation. The verifier is ~1000 lines you can read.
- Trusting Regina or SnapPy. They are excellent and they are *generators*; the
  verifier depends on neither.

## Layout

| Path | Contents |
|---|---|
| `schema/` | Normative representation, certificate, graph, sidecar, and supervision contracts |
| `verifier/` | Small independent planar-certificate verifier |
| `runtime/` | Rust proof graph, replay, lookup, routing, import, and supervision export |
| `certs/` | Human-reviewable certificate corpus |
| `tools/` | Catalogue import, witness campaigns, sidecar builders, and user CLI |
| `docs/` | Architecture, conventions, runtime design, and roadmap |
| `outputs/`, `release-v*/` | Generated local artifacts; intentionally not committed |

The repo is the source of truth. The API and website are built from it and can be
deleted and regenerated at any time.

## Status

UnknotDB is a substantial pre-release on the path to public v1, not yet a claim
of complete knot-table coverage. The core graph, compact program dictionary,
independent replay, 0-1 shortest-route recomputation, atomic snapshot
publication, invariant/identity/provenance sidecars, catalogue federation, and
graph-derived supervision exports are implemented. See
[`docs/graph-runtime-v0.md`](docs/graph-runtime-v0.md) and
[`docs/roadmap.md`](docs/roadmap.md).

The small planar verifier and the graph runtime have deliberately separate move
alphabets. Unsupported certificate primitives fail closed; a catalogue claim or
neural proposal never becomes a graph edge merely because an external tool
accepted it.

Preprocessing uses an existing,
inference-only Q254 checkpoint for `q-grown-raster-axial-12`; this project did
not train a network. The checkpoint SHA-256 is part of every production snapshot
contract, and all vertices must be reattested before a new model generation is
published. Production keys are mirror orbits: a chiral knot and its mirror share
one value node, while a replayable mirror bit transports the stored action route
back to the submitted chirality. The graph was initially populated from the
unknot by checked scrambles and has since been expanded with independently
replayed braid and planar witnesses. Synthetic data remains limited to
explicitly labelled scale benchmarks.
SQLite schema v1 stores each distinct proof program and its binary SHA-256 once;
edges contain only a compact program ID, and the validator version is
snapshot-wide metadata. Schema v2 splits the key lookup from dense hot nodes and
packs small braid alphabets at two letters per byte without changing canonical
keys. The runtime reads legacy schemas v0/v1 and can rewrite them with
`compact-snapshot INPUT OUTPUT` without rerunning the policy.
Schema v4 adds a separate exact-checkpoint key domain for replayable raw proof
vertices and a reducer-only Q254 capacity-fallback attestation. Canonical lookup
continues to use mirror/origin orbit keys; raw checkpoints never silently merge
with their mirrors.
The production braid origin/key/action convention is normative in
[`schema/graph-representation-v0.md`](schema/graph-representation-v0.md).
The mandatory initial-only reducer and normalization-only policy transitions are specified in
[`schema/preprocessing-v0.md`](schema/preprocessing-v0.md).

## Minimum-CC and L1000 routing

Semantic proof length is a separate derived objective from `U_upper`.  The
routing sidecar keeps the current minimum-CC pointer, the shortest semantic
route at that minimum CC, the scalar `1000*CC + semantic_moves` route, and a
small exact-CC frontier without changing the proof snapshot:

```bash
runtime/target/release/unknotdb-runtime build-routing-sidecar \
  proof.sqlite routing.sqlite
runtime/target/release/unknotdb-runtime show-routing \
  proof.sqlite routing.sqlite NODE_ID_OR_64_HEX_REP_KEY
```

The exact cost and tie contracts are in
[`schema/routing-sidecar-v1.md`](schema/routing-sidecar-v1.md).

## Replay-derived training data

`export-graph-supervision` turns the shortest replay-verified minimum-CC routes
into a snapshot-pinned SQLite dataset without changing the graph or policy:

```bash
runtime/target/release/unknotdb-runtime export-graph-supervision \
  proof.sqlite routing.sqlite graph-supervision.sqlite
```

The `preprocessor_labels` and `cc_solver_labels` views are disjoint tasks;
`preprocessor_stops` supplies the matching termination states without inventing
a fake semantic action. Each label retains the exact physical state in which
its anchored action is legal; all provenance occurrences and alternative
replay-valid labels are preserved.
See [`schema/graph-supervision-v1.md`](schema/graph-supervision-v1.md).

For CC-policy training, `export-cc-frontier-supervision` instead replays every
usable one-CC edge and stores a three-way judgement: all current minimum-CC
actions are accepted, completed worse routes are comparisons, and absent
actions remain unknown. Native serial controllers may use up to five internal
nonsemantic actions before the requested semantic CC. See
[`schema/cc-frontier-supervision-v0.md`](schema/cc-frontier-supervision-v0.md).

## Coherent graph release bundle

`v0.10.1` is the current coherent pre-release bundle on the path to public v1.
It contains one immutable proof graph plus five sidecars:
identification, invariant lookup, braid fingerprints, the federated
KnotInfo/Brittenham catalogue, and provenance.  Every graph-dependent sidecar
pins the exact SHA-256 of `proof.sqlite`; metadata never changes proof values.

Download the access-controlled
[Google Drive archive](https://drive.google.com/file/d/17Oc02uP1m_HREjN0Q85JvNi4zvntHEfJ/view?usp=drivesdk)
(`56,577,912` bytes = **56.58 MB** or 53.96 MiB, SHA-256
`b997f0ee9d7fdeab95f6a934a2892aefc2dae94bce76c5f5389574b5d8a48283`).

The archive expands to **229.14 MB** (218.53 MiB), including the six database
components below plus checksums, manifests, and validation reports:

| Component | Role | Size |
|---|---|---:|
| `proof.sqlite` | immutable proof graph | 29.14 MB |
| `identification.sqlite` | representation-to-knot evidence | 35.95 MB |
| `lookup.sqlite` | invariant and identifier lookup | 9.01 MB |
| `fingerprints.sqlite` | representation fingerprints | 11.34 MB |
| `federation.sqlite` | KnotInfo and Brittenham catalogue | 122.66 MB |
| `provenance.sqlite` | source and derivation records | 17.45 MB |

```bash
cd release-v0.10.1
shasum -a 256 -c SHA256SUMS
../runtime/target/release/unknotdb-runtime validate proof.sqlite
```

The full release gate and B4 regression metrics are under `reports/`. The CLI
defaults below resolve this bundle. The release archive and extracted database
bundle are generated artifacts and are not committed to Git.

## Invariant lookup CLI

The snapshot-pinned metadata sidecar is queried with `tools/unknotdb_cli.py`.
It keeps knot invariants separate from representation-dependent features.

```bash
# Values stored for a named knot or a source representation.
tools/unknotdb_cli.py invariants-for-knot 3_1
tools/unknotdb_cli.py invariants-for-representation braid:7746cc...

# Identification confidence is explicit. Candidate-only matches are never
# returned as effective knot mappings.
tools/unknotdb_cli.py identification-for-representation braid:7746cc...
tools/unknotdb_cli.py list-identification-gaps --kind candidate
tools/unknotdb_cli.py list-identification-gaps --kind unidentified

# Reverse lookup. Repeated constraints are intersected through indexed postings.
tools/unknotdb_cli.py find-knots-by-invariant \
  --invariant determinant=3 --invariant signature=2
tools/unknotdb_cli.py find-representations-by-invariant \
  --invariant determinant=3 --feature braid_strands=2

# Braid/conjugacy/policy fingerprints have explicit transformation domains.
tools/unknotdb_cli.py fingerprints-for-representation braid:7746cc...
tools/unknotdb_cli.py find-representations-by-fingerprint \
  --fingerprint braid_strands=2 --fingerprint writhe=-3

# Knot invariants and non-knot fingerprints can be intersected safely for search.
tools/unknotdb_cli.py find-representations-by-invariant \
  --invariant determinant=3 --fingerprint writhe=-3

# Diagram-dependent features are queried explicitly, never as knot invariants.
tools/unknotdb_cli.py find-representations-by-feature \
  --feature word_length=3 --feature writhe=-3

# Compute from an arbitrary braid using the pinned RF invariant implementation.
/path/to/python tools/unknotdb_cli.py compute-braid-invariants \
  --rf-src /path/to/rf-knots/src -- 2 -1,-1,-1
```

`knot_invariant`, `oriented-knot`, `rigorous_lower_bound`, and
`representation_feature` are distinct scopes. Braid word length, strand count,
writhe, policy state and learned fingerprints may be useful search features,
but they are not allowed to reject a knot match unless a separately stated
theorem makes the comparison safe.

Representation identification has three physically separate tiers:
`verified_representation_knot_map`, `attested_representation_knot_map`, and
`representation_knot_candidates`. Only the first two feed
`effective_representation_knot_map`; invariant-only candidates require new
verified or attested evidence before promotion.

The separate braid-fingerprint sidecar records `scope`, `valid_under`, and
`safe_use` for every value. Its reverse postings support candidate generation
and ranking; they are never interpreted as general knot invariants or proofs.

## Cited lower bounds

`tools/build_dgkt_lower_bounds.py` imports the corrected lower-bound results of
Dranowski--Guo--Kabkov--Tubbenhauer into a separate, provenance-bearing sidecar.
The import pins the exact commit, tree and artifact hashes from their
[computational repository](https://github.com/dtubbenhauer/unknot), names the
associated work *Machine learning methods and unknotting numbers*, and retains
both `CITATION.cff` and `ERRATUM.md` pointers. Withdrawn Owens claims are not
imported.

```bash
python3 tools/build_dgkt_lower_bounds.py \
  --source-root /clean/checkout/of/dtubbenhauer/unknot \
  --output outputs/unknotdb-dgkt-lower-bounds-v0.sqlite \
  --report outputs/unknotdb-dgkt-lower-bounds-v0-report.json

tools/unknotdb_cli.py lower-bound-for-knot 11n3
tools/unknotdb_cli.py find-knots-by-lower-bound --min 3 --exact-only
tools/unknotdb_cli.py lower-bound-sources
```

These rows are `externally_attested`, not proof-graph certificates. The importer
checks source consistency and exact certificate pointers, but promotes a claim
to `verified` only after an independent mathematical recomputation. See
[`schema/lower-bound-sidecar-v0.md`](schema/lower-bound-sidecar-v0.md).

Upper witnesses are reconstructed separately and admitted only after Rust
replay. `run_dgkt_witness_campaign.py` gives every knot an independent timeout,
durable item artifact and deterministic shard, so a hard diagram cannot block
the cohort. Exact intervals target `u_lower`; range intervals target the cited
`retained_upper` without being mislabeled as exact.

```bash
tools/run_dgkt_witness_campaign.py \
  --catalogue outputs/unknotdb-dgkt-lower-bounds-v0.sqlite \
  --graph proof.sqlite --crossings 13 --interval-kind exact \
  --output-dir outputs/dgkt/items --combined-output outputs/dgkt/shard0.json \
  --manifest outputs/dgkt/shard0.jsonl --worker-python /path/to/python \
  --per-knot-timeout 30 --shard-count 4 --shard-index 0

runtime/target/release/unknotdb-runtime import-catalogue-planar \
  proof.sqlite proof-with-dgkt.sqlite --corpus outputs/dgkt/shard0.json \
  --manifest outputs/dgkt-import.tsv --oracle /path/to/python \
  tools/q_policy_oracle.py --pgx-root /path/to/pgx-mcts-bench \
  --model-dir models/q-grown-raster-axial-12-q254-frozen-20260824-v0
```

Every multi-CC witness becomes a chain of one-CC edges. Mandatory preprocessing
and normalization run after each CC; the import manifest records the resulting
source stopping key. `promote_dgkt_witness_identifications.py` then attaches the
named knot as `attested` identity evidence, while the graph route itself remains
fully replay-verified. `build_dgkt_witness_coverage_report.py` separates covered,
mapped-but-too-long and unmapped claims for the next targeted search.

## Federated catalogue and CC adjacency

`tools/build_federated_catalogue.py` imports a hash-pinned KnotInfo snapshot
into a separate searchable layer.  The proof graph remains small: public DT,
Gauss, PD and braid representations are aliases until a replay-validated route
actually connects a normalized stopping point.

```bash
tools/unknotdb_cli.py knot-show 12n_570
tools/unknotdb_cli.py resolve-identifier K12n570 --scheme spherogram
tools/unknotdb_cli.py resolve-representation dt '[4, 6, 2]'
tools/unknotdb_cli.py pd-for-graph HEX_OR_NODE_ID
tools/unknotdb_cli.py graph-for-pd '[[1,5,2,4],[...]]'
tools/unknotdb_cli.py find-knots-by-catalogue-property \
  --property unknotting_number=1
tools/unknotdb_cli.py neighbors 9_19 --status replay_verified
tools/unknotdb_cli.py catalogue-coverage
```

`pd-for-graph` and `graph-for-pd` join the identification and federated
catalogue sidecars through the stable knot ID. Their rows say
`relation=same_knot_type` and `exact_conversion=false` unless a future replayed
diagram-conversion relation supports the stronger claim; a knot-name mapping
is never presented as a specific PD-to-braid isotopy certificate.

Adjacency rows are explicitly `claimed`, `diagram_attested`, or
`replay_verified`.  Only the last status is a named projection of an immutable
one-CC proof edge in a pinned graph snapshot.  See
[`schema/federated-catalogue-v1.md`](schema/federated-catalogue-v1.md).

Jones coverage is complete for the 3,401 named knots in the current maps. Wide
source braids are handled by `tools/backfill_jones_pd.py`: it checks the exact
Spherogram source-braid identity, then evaluates the equivalent minimal
11--13-crossing PD diagram by an exact Kauffman state sum. This avoids the
Temperley--Lieb/Catalan blow-up caused by computing directly on a wide braid.

## Embedding supervision export

Unknot DB exports replay-derived interval labels without changing the proof
graph or a policy checkpoint. It owns the evidence: representations, distances,
witness lengths, split units, provenance, and schema versions. Neural models,
losses, data loaders, training loops, checkpoints, and evaluation live in the
sibling [RF Knots](https://github.com/avorozhtsov/rf-knots) project.

```bash
runtime/target/release/unknotdb-runtime export-embedding-pairs \
  proof.sqlite identification.sqlite embedding-pairs.sqlite
```

Pair labels are intervals: a replayed path is only an upper bound unless the
exporter also certifies the lower bound. Identity/orbit-disjoint split units
prevent known equivalent representations from leaking between train and
evaluation. See [`schema/embedding-pairs-v0.md`](schema/embedding-pairs-v0.md).

## Naming

`unknotdb` — free on PyPI, crates.io and npm, and unambiguous in search. The
binary, the crate and the repo all share the name.

## License

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your
option.
