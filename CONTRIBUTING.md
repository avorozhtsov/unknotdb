# Contributing

Submissions are pull requests. CI is the referee: it re-runs the verifier over
what you changed and merges nothing it cannot check.

## Submitting a certificate

Generate it with the tool rather than writing the trace by hand:

```bash
cargo build --release --manifest-path verifier/Cargo.toml
./verifier/target/release/unknotdb mkcert "braid:2:1,1,1" \
    --knotinfo 3_1 --source "Rolfsen 1976" --date 2026-08-15 > /tmp/k.cert
./verifier/target/release/unknotdb verify /tmp/k.cert
mkdir -p "$(dirname "$(./verifier/target/release/unknotdb id /tmp/k.cert)")"
mv /tmp/k.cert "$(./verifier/target/release/unknotdb id /tmp/k.cert)"
```

Certificates produced by other tooling are equally welcome — the verifier does
not care who wrote the trace, and accepts moves (`R2+`, in particular) that the
built-in search never emits. The format is
[`schema/certificate-format.md`](schema/certificate-format.md).

A pull request is accepted when:

- `unknotdb verify` passes on every certificate it adds or changes;
- the filename is the content-addressed one `unknotdb id` reports;
- the bound is at least as good as the one already in the corpus — a strictly
  better bound must set `supersedes` to the sha256 of the record it replaces;
- `claim_source` and `witness_author` are both filled in, and they are
  different things: who first asserted the bound, and who produced this
  witness;
- `assumes` is present. Write `[]` if the claim is unconditional. Omitting it
  is an error, not a default.

Certificates are never edited in place to weaken a bound. Supersede the old
record and leave it in the history.

## Conditional claims

If a claim depends on an unproven conjecture, say so in `assumes`, drawing from
`schema/assumptions.yaml`. A conditional claim is served as conditional and can
be retracted mechanically if the assumption falls. This is not a formality: the
43 newly-determined `<= 12`-crossing unknotting numbers in arXiv:2409.09032 rest
on additivity of `u`, and later work by the same group put that under strain.

## Changing the verifier

- No dependencies. This is a design constraint, not an oversight — the verifier
  has to be readable end to end and must not inherit trust from Regina, SnapPy,
  or a YAML library.
- A move the verifier cannot fully check must return an error. Never accept a
  move on the grounds that it looks right; `R2+`, `R3` and the unimplemented
  alphabets all follow this rule.
- Construct, validate, reject. Every move re-validates planarity on its result.
  If a construction produces an invalid diagram, that is a rejection, never
  something to patch up.
- New moves need ground truth from outside the implementation. R3 is checked
  against the braid relation with both sides built independently; R2+ was
  pinned down by an exhaustive scan over candidate wirings. "It type-checks and
  the diagram looks plausible" is not evidence. See
  [`docs/roadmap.md`](docs/roadmap.md) for how each was established.
- `cargo test` and `cargo fmt --check` must pass.

## Lower bounds

There is no such thing as a certified lower bound — no move sequence witnesses
`u(K) >= 3`. Lower bounds are `cited`, with tool, version and input hash for
reproducibility. Please do not submit them as certificates; the value is in
pairing a certified upper bound with a cited lower bound that meets it.
