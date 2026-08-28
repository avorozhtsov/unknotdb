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
- `build_federated_catalogue.py` imports a hash-pinned KnotInfo XLS snapshot,
  compact public representations and indexed hot properties into a proof-
  external SQLite sidecar.  Optional adjacency TSVs retain source metadata and
  trust status; named graph adjacency is imported only from replay-validated
  one-CC edges in a pinned snapshot.
- `import_brittenham_adjacency.py` streams the published 12/13-crossing ZIPs,
  filters all-subset output down to exact one-crossing rows modulo mirror, and
  atomically adds fixed-diagram claims without changing the proof graph.
- `audit_brittenham_b5.py` exhausts the 816 apparent ways to append three
  crossing changes to the two published changes in the original 20-letter B5
  chart.  It records the exact Fox-colouring determinant of every result; this
  prevents confusing the paper's diagram-changing five-step route with a
  nonexistent five-dimensional crossing cube in one braid chart.
- `build_graph_identification_sidecar.py` builds graph-wide knot-ID postings,
  propagates non-conflicting provenance labels through replay-validated CC=0
  components, and creates a bounded candidate queue from exact invariant
  bundles plus a shared optimal one-CC target.
- `check_identification_equivalence.py` checks that queue with bounded Regina
  Reidemeister/complement tests in a separate environment. Successful rows are
  external attestations, not proof-graph certificates. `promote_regina_unknots.py`
  separately records zero-crossing simplifications of previously unidentified
  source representations.
- `attest_named_proof_chain.py` binds published knot names to states of an
  already replay-verified graph chain. It checks pinned graph nodes, active
  one-CC edges, source hashes and SnapPy isometry against the published DT
  diagrams before atomically adding forward and reverse postings.
- `unknotdb_cli.py` queries the invariant, identification, fingerprint and
  federated catalogue layers through one interface (`knot-show`, identifier or
  representation resolution, reverse property lookup, adjacency and coverage).
- `prepare_release_identification.py`, `build_provenance_sidecar.py`, and
  `build_release_bundle.py` repin the metadata layers, preserve only known
  source associations, and atomically assemble a checksum-addressed coherent
  release. `cleanup_superseded_sqlite.py` deletes only its audited exact list of
  generated SQLite paths after that release and archive have passed validation.
