# unknotdb

**Machine-checked certificates for knot theory.**

Scope note: despite the name, the certificate machinery is not specific to
unknotting. The same verifier covers slice genus, Gordian distance, braid index
and unlinking — see [`schema/claim-types.md`](schema/claim-types.md).

Every unknotting-number bound in the literature is currently an assertion plus a
citation. KnotInfo stores `[2,3]` in a cell and links to a paper; the paper prints
a PD code in a LaTeX appendix. Nothing is replayable.

unknotdb stores the **witness**: an explicit sequence of moves that a small,
dependency-free program re-checks from scratch. Certificates live in a git repo,
submissions are pull requests, and CI is the referee.

## Quickstart

No dependencies; a Rust toolchain is all you need.

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

`unknotdb help` lists the rest. To submit, see
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

```
schema/     certificate + claim format, conventions        (normative)
verifier/   the Rust verifier crate; binary `unknotdb`
runtime/    generated SQLite snapshots + in-memory hot lookup
certs/      the corpus, one file per claim
tools/      Python: ingest, generators, site build
docs/       design notes
site/       generated; disposable
```

The repo is the source of truth. The API and website are built from it and can be
deleted and regenerated at any time.

## Status

Early. Alphabets `R` (R1±, R2±, R3) and `X` (plus crossing changes) are
complete. Markov and band moves are specified but not implemented, and are
rejected rather than trusted. See [`docs/roadmap.md`](docs/roadmap.md).

The standalone graph runtime is now prototyped separately from the verifier; see
[`docs/graph-runtime-v0.md`](docs/graph-runtime-v0.md). Its benchmark generator
uses synthetic, explicitly non-proof edges. Preprocessing uses an existing,
inference-only Q254 checkpoint for `q-grown-raster-axial-12`; this project did
not train a network. The checkpoint SHA-256 is part of every production snapshot
contract, and all vertices must be reattested before a new model generation is
published. Production keys are mirror orbits: a chiral knot and its mirror share
one value node, while a replayable mirror bit transports the stored action route
back to the submitted chirality. The first real snapshot is populated from the
unknot by a checked scramble and monotone Bellman relaxation; synthetic data is
still used only for scale benchmarks.
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

## Coherent graph release bundle

`v0.10.1` is the current coherent pre-release bundle on the path to public v1.
It contains one immutable proof graph plus five sidecars:
identification, invariant lookup, braid fingerprints, the federated
KnotInfo/Brittenham catalogue, and provenance.  Every graph-dependent sidecar
pins the exact SHA-256 of `proof.sqlite`; metadata never changes proof values.

Download the access-controlled
[Google Drive archive](https://drive.google.com/file/d/17Oc02uP1m_HREjN0Q85JvNi4zvntHEfJ/view?usp=drivesdk)
(`56,577,912` bytes, SHA-256
`b997f0ee9d7fdeab95f6a934a2892aefc2dae94bce76c5f5389574b5d8a48283`).

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

## Federated catalogue and CC adjacency

`tools/build_federated_catalogue.py` imports a hash-pinned KnotInfo snapshot
into a separate searchable layer.  The proof graph remains small: public DT,
Gauss, PD and braid representations are aliases until a replay-validated route
actually connects a normalized stopping point.

```bash
tools/unknotdb_cli.py knot-show 12n_570
tools/unknotdb_cli.py resolve-identifier K12n570 --scheme spherogram
tools/unknotdb_cli.py resolve-representation dt '[4, 6, 2]'
tools/unknotdb_cli.py find-knots-by-catalogue-property \
  --property unknotting_number=1
tools/unknotdb_cli.py neighbors 9_19 --status replay_verified
tools/unknotdb_cli.py catalogue-coverage
```

Adjacency rows are explicitly `claimed`, `diagram_attested`, or
`replay_verified`.  Only the last status is a named projection of an immutable
one-CC proof edge in a pinned graph snapshot.  See
[`schema/federated-catalogue-v1.md`](schema/federated-catalogue-v1.md).

Jones coverage is complete for the 3,401 named knots in the current maps. Wide
source braids are handled by `tools/backfill_jones_pd.py`: it checks the exact
Spherogram source-braid identity, then evaluates the equivalent minimal
11--13-crossing PD diagram by an exact Kauffman state sum. This avoids the
Temperley--Lieb/Catalan blow-up caused by computing directly on a wide braid.

## Naming

`unknotdb` — free on PyPI, crates.io and npm, and unambiguous in search. The
binary, the crate and the repo all share the name.

## License

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your
option.
