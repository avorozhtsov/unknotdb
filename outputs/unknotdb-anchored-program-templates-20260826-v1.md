# Anchored proof-program templates, schema v3

## Implemented contract

An edge now binds a coordinate-independent proof template to a concrete torus
location:

```text
programs(program_id, program_version, relative_template, template_sha256)
edges(..., program_id, program_anchor_x, program_anchor_y, first_action, ...)
```

`program_id` identifies the relative transformation and excludes the anchor.
`program_anchor_y` is the exact position of the first position-bearing semantic
operation. `program_anchor_x` is its generator coordinate. `first_action`
remains a separate instantiated hot-routing cache and is not part of template
identity.

The first local operation has relative position zero. Later positions are
encoded relative to the same anchor, reduced in the current action's coordinate
domain. Materialization replays statefully, so word-length changes caused by
`INSERT`, `REDUCE`, and stabilization are handled exactly. Origin instructions
remain explicit proof instructions. The validator rejects noncanonical anchors,
checks `anchor_x` against the first concrete operation, materializes the exact
program, recomputes CC cost, and replays to the stored target.

The snapshot records binding codec
`unknotdb-relative-anchor-program-v1`. Schema-v2 snapshots remain readable;
`compact-snapshot` migrates them atomically to schema v3.

## Measured full-graph result

Parent snapshot: `unknotdb-rf-complete-audited-v0.sqlite`.

| Metric | Exact programs, v2 | Anchored templates, v3 |
|---|---:|---:|
| Nodes | 94,642 | 94,642 |
| Edges | 146,940 | 146,940 |
| Dictionary entries | 20,529 | 15,268 |
| Program/template blob bytes | 1,135,836 | 921,312 |
| Hash bytes | 656,928 | 488,576 |
| Average edges per entry | 7.158 | 9.624 |
| Maximum uses | 14,320 | 49,659 |
| Snapshot bytes | 24,420,352 | 25,952,256 |

Coordinate factoring merged 5,261 dictionary entries (25.6%) and removed
214,524 blob bytes plus 168,352 hash bytes. The complete file is 1,531,904
bytes larger because the new `edges_by_program(program_id)` occurrence-search
index alone occupies 1,515,520 bytes, and every edge now carries explicit
anchors. This is a deliberate pattern-search index, not a compression claim.

The graph uses 217 distinct `(anchor_x, anchor_y)` pairs. Examples of recovered
typical transformations:

- `[CC(relative=0)]`: 49,659 edges at 190 distinct anchors.
- `[CC(0), NormalizeOrigin(2)]`: 6,242 edges.
- `[CC(0), Reduce(0)]`: 3,902 edges at 83 distinct anchors.

## Search interface

Rank templates by occurrence count and emit human-readable relative programs:

```text
unknotdb-runtime report-program-templates SNAPSHOT \
  --report REPORT.tsv --limit 100
```

Find concrete applications of one template through the indexed reverse lookup:

```text
unknotdb-runtime program-template-occurrences SNAPSHOT TEMPLATE_ID --limit 100
```

Each occurrence reports source, target, `anchor_x`, `anchor_y`, CC cost and the
instantiated first action.

## Validation

- Final snapshot SHA-256:
  `47f84db3a0ab24ce7846c5f12f1b85000cda33d015a702305a84dc00532f328f`.
- SQLite integrity and foreign-key checks passed.
- Full replay materialized and independently verified all 146,940 bindings.
- Frozen B4 coverage remained 286,334/286,334.
- A schema-v3 -> schema-v3 full round trip had zero SQL deltas in nodes, keys,
  representations, programs, edges and policy stops.
- 62 Rust unit tests and the RF reference-vector test passed.
- Formatting and strict Clippy passed.

## Artifacts

- Snapshot: `outputs/unknotdb-rf-complete-anchored-schema3-v1.sqlite`
- Top-template report: `outputs/unknotdb-program-templates-top100-v1.tsv`
- B4/full-replay gate: `outputs/unknotdb-rf-complete-anchored-schema3-b4-v1.tsv`
- Round-trip snapshot:
  `outputs/unknotdb-rf-complete-anchored-schema3-roundtrip-v1.sqlite`
