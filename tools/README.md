# tools

Python helpers: ingest, generators, site build and the external inference
boundary. Nothing here is trusted by the verifier.

- `freeze_q_policy_checkpoint.py` extracts one scientist network from a single
  immutable read of a Q state and writes a hash-pinned inference checkpoint plus
  provenance manifest. It does not train or modify the source experiment.
- `q_policy_oracle.py` serves deterministic top-1 decisions from that frozen
  checkpoint. It may advance only internal controller state; Rust owns semantic
  replay, bounds and stopping-point acceptance. A bounded clean-controller LRU
  avoids repeated raster/network work for an identical normalized input;
  `--decision-cache-size 0` disables it. `--profile-timings-every N` emits
  cumulative phase measurements to stderr without changing the line protocol.
- `generate_rf_reference_fixture.py` generated the frozen cross-implementation
  semantic vectors. RF Knots is read-only input to this helper.
- `extract_rf_representation_corpus.py` reads the pinned benchmark, evidence and
  catalogue files in a read-only RF Knots checkout and emits a versioned,
  source-hashed braid corpus. Canonicalisation and preprocessing are intentionally
  deferred to the pinned Unknot DB ingestion contract.
