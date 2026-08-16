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

## Naming

`unknotdb` — free on PyPI, crates.io and npm, and unambiguous in search. The
binary, the crate and the repo all share the name.

## License

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your
option.
