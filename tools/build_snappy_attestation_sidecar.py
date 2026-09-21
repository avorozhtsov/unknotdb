#!/usr/bin/env python3
"""SnapPy census identification of unlabelled CC=0 components, as attestations.

SnapPy 3.3.2 can name a hyperbolic knot complement by matching it against the
bundled censuses.  That is an *external* computation, so everything this tool
writes carries the trust class `attested`: it never becomes `verified`, it never
creates a proof-graph edge, and it never turns a catalogue `u` into a theorem.

The tool is additive.  It copies its parent identification sidecar, bumps the
schema, and adds `snappy_*` tables.  Nothing existing is deleted or rewritten
except `graph_vertex_knot_map`, which gains `attested` rows for components that
passed every gate below.

Gates for writing a label into `graph_vertex_knot_map`:

1. the braid closure is a knot (one link component);
2. `identify()` resolved to exactly one catalogue identity -- ambiguous rows are
   recorded in full and never collapsed to a first match;
3. the four invariants the schema's bounded-candidate rule already uses
   (determinant, Alexander, Jones mirror orbit, absolute signature) agree
   between the graph representative and the catalogue braid for that name;
4. the numerical isometry match is reproduced at 212-bit precision.

Disagreements are quarantined in `snappy_quarantine`, never dropped: either
SnapPy misidentified the complement or a stored invariant is wrong, and both are
worth knowing.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sqlite3
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build_graph_identification_sidecar import (
    add_evidence,
    canonical_json,
    decode_representation,
    file_sha256,
)

SCHEMA = "unknotdb-graph-identification-v6"
METHOD = "snappy-census-identify"
FINGERPRINT_METHOD = "rf-knot-table-fingerprint"

SCHEMA_SQL = """
CREATE TABLE snappy_census_identifications(
    rep_key BLOB PRIMARY KEY CHECK(length(rep_key)=32),
    cc0_component_key BLOB NOT NULL,
    node_id INTEGER NOT NULL,
    prior_knot_id TEXT,
    status TEXT NOT NULL,
    method TEXT NOT NULL,
    evidence_class TEXT NOT NULL CHECK(evidence_class='attested'),
    link_components INTEGER,
    crossings_in INTEGER NOT NULL,
    crossings_out INTEGER,
    solution_type TEXT,
    volume REAL,
    rigor TEXT NOT NULL,
    isometry_signature TEXT,
    hp_isometry_signature TEXT,
    census_names_json TEXT NOT NULL,
    canonical_ids_json TEXT NOT NULL,
    unresolved_names_json TEXT NOT NULL,
    ambiguous INTEGER NOT NULL CHECK(ambiguous IN (0,1)),
    ambiguity_kind TEXT,
    cross_check TEXT NOT NULL,
    fingerprint_names_json TEXT NOT NULL,
    fingerprint_verdict TEXT NOT NULL,
    labelled INTEGER NOT NULL,
    label_method TEXT,
    elapsed_ms INTEGER NOT NULL,
    FOREIGN KEY(rep_key) REFERENCES graph_vertices(rep_key)
) WITHOUT ROWID;
CREATE INDEX snappy_ident_by_status ON snappy_census_identifications(status,rep_key);
CREATE INDEX snappy_ident_by_component
    ON snappy_census_identifications(cc0_component_key);

CREATE TABLE snappy_identification_names(
    rep_key BLOB NOT NULL,
    census_name TEXT NOT NULL,
    name_kind TEXT NOT NULL,
    canonical_id TEXT,
    PRIMARY KEY(rep_key,census_name),
    FOREIGN KEY(rep_key) REFERENCES snappy_census_identifications(rep_key)
) WITHOUT ROWID;
CREATE INDEX snappy_names_by_canonical
    ON snappy_identification_names(canonical_id,rep_key);

CREATE TABLE snappy_invariant_crosschecks(
    rep_key BLOB NOT NULL,
    canonical_id TEXT NOT NULL,
    verdict TEXT NOT NULL,
    disagreeing_fields TEXT,
    query_determinant TEXT NOT NULL,
    query_absolute_signature TEXT NOT NULL,
    query_bundle_sha256 BLOB NOT NULL CHECK(length(query_bundle_sha256)=32),
    reference_determinant TEXT,
    reference_absolute_signature TEXT,
    reference_bundle_sha256 BLOB,
    PRIMARY KEY(rep_key,canonical_id),
    FOREIGN KEY(rep_key) REFERENCES snappy_census_identifications(rep_key)
) WITHOUT ROWID;
CREATE INDEX snappy_crosschecks_by_verdict
    ON snappy_invariant_crosschecks(verdict,rep_key);

CREATE TABLE snappy_quarantine(
    rep_key BLOB NOT NULL,
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    PRIMARY KEY(rep_key,reason),
    FOREIGN KEY(rep_key) REFERENCES snappy_census_identifications(rep_key)
) WITHOUT ROWID;

CREATE TABLE snappy_connected_sum_decompositions(
    rep_key BLOB NOT NULL,
    summand_index INTEGER NOT NULL,
    summand_braid_json TEXT NOT NULL,
    summand_status TEXT NOT NULL,
    summand_names_json TEXT NOT NULL,
    PRIMARY KEY(rep_key,summand_index),
    FOREIGN KEY(rep_key) REFERENCES snappy_census_identifications(rep_key)
) WITHOUT ROWID;

CREATE TABLE snappy_candidate_resolutions(
    candidate_id BLOB PRIMARY KEY CHECK(length(candidate_id)=32),
    left_rep_key BLOB NOT NULL,
    candidate_knot_id TEXT NOT NULL,
    snappy_status TEXT NOT NULL,
    snappy_canonical_ids_json TEXT NOT NULL,
    verdict TEXT NOT NULL,
    FOREIGN KEY(candidate_id) REFERENCES graph_equivalence_candidates(candidate_id)
) WITHOUT ROWID;

CREATE TABLE snappy_prior_label_agreement(
    cc0_component_key BLOB PRIMARY KEY CHECK(length(cc0_component_key)=32),
    prior_knot_id TEXT NOT NULL,
    prior_evidence_class TEXT NOT NULL,
    snappy_status TEXT NOT NULL,
    snappy_canonical_ids_json TEXT NOT NULL,
    verdict TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX snappy_prior_agreement_by_verdict
    ON snappy_prior_label_agreement(verdict);

CREATE TABLE snappy_u_upper_tightenings(
    rep_key BLOB PRIMARY KEY CHECK(length(rep_key)=32),
    node_id INTEGER NOT NULL,
    prior_u_upper INTEGER NOT NULL,
    attested_u_upper INTEGER NOT NULL,
    method TEXT NOT NULL,
    evidence_class TEXT NOT NULL CHECK(evidence_class='attested'),
    crossings_in INTEGER NOT NULL,
    FOREIGN KEY(rep_key) REFERENCES snappy_census_identifications(rep_key)
) WITHOUT ROWID;
CREATE INDEX snappy_tightenings_by_gap
    ON snappy_u_upper_tightenings(prior_u_upper DESC,node_id);

CREATE VIEW snappy_attested_postings AS
    SELECT n.canonical_id AS knot_id, lower(hex(i.rep_key)) AS representation_ref,
           i.node_id, i.cc0_component_key, i.status, i.rigor, i.cross_check,
           i.ambiguous, 'attested' AS evidence_class
    FROM snappy_identification_names n
    JOIN snappy_census_identifications i USING(rep_key)
    WHERE n.canonical_id IS NOT NULL;
"""


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--report-json", required=True, type=Path)
    parser.add_argument("--federation", required=True, type=Path)
    parser.add_argument("--pipeline-dir", required=True, type=Path)
    parser.add_argument("--deadline-seconds", type=int, default=5400)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--progress-seconds", type=float, default=15.0)
    args = parser.parse_args()
    if args.output.exists() or args.report.exists():
        raise FileExistsError("output artifact already exists")

    started = time.monotonic()
    deadline = started + args.deadline_seconds
    sys.path.insert(0, str(args.pipeline_dir))
    import snappy
    from pipeline import ReferenceBundles, classify_name, identify_item
    from rf_knots import invariants as inv
    from rf_knots import knot_table, seifert

    engine = {
        "snappy_version": snappy.__version__,
        "python": sys.version.split()[0],
        "method": METHOD,
        "identify_rigor": "numerical-hyperbolic-census-match",
        "non_hyperbolic_fallback": "rf-knot-table-fingerprint "
        "(determinant, Jones; Alexander tie-break; reports name sets)",
        "verified_hyperbolicity": "unavailable-requires-sage",
        "strengthening": "212-bit-high-precision-recomputation-and-isometry-signature",
        "determinant_and_signature_backend": "rf_knots.seifert (SnapPy's own "
        "determinant()/signature() raise SageNotAvailable outside Sage)",
    }
    print(f"engine: {canonical_json(engine)}", flush=True)

    input_sha256 = file_sha256(args.input)
    temp = args.output.with_name(args.output.name + f".tmp-{os.getpid()}")
    shutil.copyfile(args.input, temp)
    db = sqlite3.connect(temp)
    db.execute("PRAGMA foreign_keys=ON")
    schema = db.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
    if schema not in {("unknotdb-graph-identification-v4",),
                      ("unknotdb-graph-identification-v5",)}:
        raise ValueError(f"unsupported input schema: {schema}")
    db.executescript(SCHEMA_SQL)

    # ---- component representatives ---------------------------------------
    print("loading graph vertices ...", flush=True)
    best: dict[bytes, dict] = {}
    decode_failures = 0
    for rep_key, node_id, encoding, component, u_upper in db.execute(
        "SELECT rep_key,node_id,encoding,cc0_component_key,u_upper FROM graph_vertices"
    ):
        rep_key, component = bytes(rep_key), bytes(component)
        try:
            strands, cyclic, word = decode_representation(bytes(encoding))
        except Exception:  # noqa: BLE001 - a bad encoding must not abort the sweep
            decode_failures += 1
            continue
        order = (len(word), strands, rep_key)
        current = best.get(component)
        if current is None or order < current["order"]:
            best[component] = {"order": order, "rep_key": rep_key,
                               "node_id": int(node_id), "strands": strands,
                               "cyclic": cyclic, "word": word,
                               "component": component, "u_upper": u_upper}
    prior: dict[bytes, tuple[str, str]] = {}
    for component, knot_id, evidence_class in db.execute(
        """SELECT v.cc0_component_key,m.knot_id,m.evidence_class
           FROM graph_vertex_knot_map m JOIN graph_vertices v USING(rep_key)"""
    ):
        # stored as `knot:NAME`; compared against SnapPy's catalogue names
        prior.setdefault(bytes(component),
                         (str(knot_id).removeprefix("knot:"), str(evidence_class)))
    # `knot_id` is uniformly `knot:` + `canonical_name` in every existing sidecar.
    known_knot_names = {
        str(n) for (n,) in db.execute("SELECT canonical_name FROM knot_ids")}

    def knot_id_of(name: str) -> str:
        return f"knot:{name}"

    def name_of(knot_id: str) -> str:
        return knot_id.removeprefix("knot:")
    pending_left = {
        bytes(left): (bytes(cid), str(knot).removeprefix("knot:"))
        for cid, left, knot in db.execute(
            """SELECT candidate_id,left_rep_key,candidate_knot_id
               FROM graph_equivalence_candidates WHERE status='pending'""")
    }
    print(f"components={len(best):,} prior_labelled={len(prior):,} "
          f"decode_failures={decode_failures} pending_candidates={len(pending_left)} "
          f"({time.monotonic()-started:.1f}s)", flush=True)

    refs = ReferenceBundles(inv, seifert, args.federation)
    print(f"reference catalogue: {len(refs.words):,} braid words, "
          f"{len(refs.aliases):,} usable aliases, "
          f"{len(refs.alias_collisions):,} colliding identifiers dropped", flush=True)

    # unlabelled first, so a deadline cuts into the least valuable work last
    order = sorted(best.values(), key=lambda r: (r["component"] in prior, r["order"]))
    if args.limit:
        order = order[: args.limit]

    stats: Counter[str] = Counter()
    cross_stats: Counter[str] = Counter()
    fingerprint_stats: Counter[str] = Counter()
    rigor_stats: Counter[str] = Counter()
    by_crossings: dict[int, Counter[str]] = defaultdict(Counter)
    quarantined = 0
    ambiguous_rows = 0
    labelled_components = 0
    labelled_vertices = 0
    new_knot_ids = 0
    processed = 0
    total_seconds = 0.0
    invariant_seconds = 0.0
    truncated = False
    last = time.monotonic()

    def evidence_for(component: bytes, canonical_id: str, row: dict[str, Any],
                     method: str) -> bytes:
        """One evidence row per (knot, method, rigour, cross-check) claim.

        Deliberately *not* per component.  A per-component row carrying a copy of
        the engine dict cost 362 MB for 79k components in the first build, and
        added nothing: the component -> evidence link is already carried by
        `graph_vertex_knot_map.evidence_id`, and the full census-name list for
        each vertex is in `snappy_identification_names`.  The engine is recorded
        once, in `meta`.
        """
        del component
        pointer = (f"method={method};knot_id={canonical_id};"
                   f"rigor={row['rigor']};cross_check={row['cross_check']}")
        return add_evidence(
            db, "attested", method, str(args.input), input_sha256, pointer,
            {
                "knot_id": canonical_id,
                "method": method,
                "rigor": row["rigor"],
                "cross_check": row["cross_check"],
                "engine_ref": "meta:snappy_attestation_engine",
                "trust": "external-snappy-attestation-not-proof-graph-replay",
            },
        )

    for record in order:
        if time.monotonic() >= deadline:
            truncated = True
            print(f"DEADLINE reached after {processed:,} items", flush=True)
            break
        component = record["component"]
        row = identify_item(record["word"], snappy=snappy, inv=inv,
                            seifert=seifert, refs=refs, knot_table=knot_table)
        processed += 1
        total_seconds += row["seconds"]
        invariant_seconds += row["seconds_invariants"]
        stats[row["status"]] += 1
        cross_stats[row["cross_check"]] += 1
        fingerprint_stats[row["fingerprint_verdict"]] += 1
        rigor_stats[row["rigor"]] += 1
        crossings = row["crossings_out"] if row["crossings_out"] is not None else -1
        by_crossings[crossings][row["status"]] += 1
        ambiguous_rows += row["ambiguous"]

        prior_knot = prior.get(component)
        canonical_ids = row["canonical_ids"]
        identities = canonical_ids + row["unresolved_knot_names"]
        label_method = None
        if row["status"] == "unknot-diagram":
            gate_ok, label = True, "0_1"
            label_method = "snappy-global-simplify-to-zero-crossings"
        elif row["status"] == "identified":
            gate_ok = (
                not row["ambiguous"]
                and row["link_components"] == 1
                and len(identities) == 1
                and row["cross_check"] in {"agree", "no-reference-braid"}
                and row["rigor"] == "numerical-match-confirmed-at-212-bit-precision"
            )
            label = identities[0] if len(identities) == 1 else None
            label_method = METHOD
        elif row["fingerprint_verdict"] == "unique":
            # SnapPy is silent on non-hyperbolic knots; the bundled table is not.
            gate_ok = row["link_components"] == 1
            label = row["fingerprint_names"][0]
            label_method = FINGERPRINT_METHOD
        else:
            gate_ok, label = False, None
        will_label = bool(gate_ok and label and prior_knot is None)

        db.execute(
            "INSERT INTO snappy_census_identifications VALUES "
            "(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                record["rep_key"], component, record["node_id"],
                prior_knot[0] if prior_knot else None,
                row["status"], METHOD, "attested",
                row["link_components"], row["crossings_in"], row["crossings_out"],
                row["solution_type"], row["volume"], row["rigor"],
                row["isometry_signature"], row["hp_isometry_signature"],
                canonical_json(row["names"]), canonical_json(canonical_ids),
                canonical_json(row["unresolved_knot_names"]),
                row["ambiguous"], row["ambiguity_kind"], row["cross_check"],
                canonical_json(row["fingerprint_names"]), row["fingerprint_verdict"],
                int(will_label), label_method if will_label else None,
                round(row["seconds"] * 1000),
            ),
        )
        for name in row["names"]:
            kind, knot_name = classify_name(name)
            db.execute(
                "INSERT OR IGNORE INTO snappy_identification_names VALUES (?,?,?,?)",
                (record["rep_key"], name, kind,
                 refs.resolve(knot_name) if knot_name else None),
            )
        if row["status"] == "unknot-diagram" and not row["names"]:
            db.execute(
                "INSERT OR IGNORE INTO snappy_identification_names VALUES (?,?,?,?)",
                (record["rep_key"], "0_1", "knot", "0_1"))

        query = row["query_bundle"]
        if query and row["reference_bundles"]:
            for name, reference in row["reference_bundles"].items():
                canonical = refs.resolve(name) or name
                verdict = "no-reference"
                fields = None
                if reference and "error" not in reference:
                    index = row["knot_names"].index(name)
                    detail = row["cross_check_detail"][index]
                    verdict = "agree" if detail == "agree" else "disagree"
                    fields = None if verdict == "agree" else detail.split(":", 1)[1]
                db.execute(
                    "INSERT OR IGNORE INTO snappy_invariant_crosschecks VALUES "
                    "(?,?,?,?,?,?,?,?,?,?)",
                    (
                        record["rep_key"], canonical, verdict, fields,
                        query["determinant"], query["absolute_signature"],
                        bytes.fromhex(query["sha256"]),
                        (reference or {}).get("determinant"),
                        (reference or {}).get("absolute_signature"),
                        bytes.fromhex(reference["sha256"])
                        if reference and "sha256" in reference else None,
                    ),
                )

        # Only genuine connected sums; a one-piece decomposition means "prime".
        for index, factor in enumerate(row["summand_names"] or []):
            names = factor.get("names") or factor.get("fingerprint_names") or []
            db.execute(
                "INSERT OR IGNORE INTO snappy_connected_sum_decompositions "
                "VALUES (?,?,?,?,?)",
                (record["rep_key"], index, canonical_json(factor.get("braid", [])),
                 factor.get("status", "unknown"), canonical_json(names)),
            )

        # ---- quarantine -------------------------------------------------
        reasons: list[tuple[str, dict[str, Any]]] = []
        if row["cross_check"] == "disagreement":
            reasons.append(("invariant-disagreement", {
                "census_names": row["names"], "detail": row["cross_check_detail"],
                "query_bundle": query, "reference_bundles": row["reference_bundles"],
                "note": "either a SnapPy misidentification or a wrong stored "
                        "invariant; not used for any label",
            }))
        if row["fingerprint_verdict"] == "ambiguous":
            # e.g. 5_1 and 10_132 agree on Jones AND Alexander but have
            # different unknotting numbers -- a set, not a knot.
            reasons.append(("ambiguous-fingerprint-match", {
                "fingerprint_names": row["fingerprint_names"],
                "notes": row["fingerprint_notes"],
                "status": row["status"],
            }))
        if row["ambiguous"]:
            reasons.append(("ambiguous-census-match", {
                "census_names": row["names"], "canonical_ids": canonical_ids,
                "unresolved": row["unresolved_knot_names"],
                "kind": row["ambiguity_kind"],
            }))
        if row["status"] == "error" or row["cross_check"] == "error":
            reasons.append(("computation-error", {
                "error": row["error"], "cross_check_detail": row["cross_check_detail"]}))
        if row["link_components"] not in (None, 1):
            reasons.append(("not-a-knot", {
                "link_components": row["link_components"]}))
        if prior_knot and canonical_ids and prior_knot[0] not in canonical_ids \
                and row["status"] == "identified":
            reasons.append(("disagrees-with-prior-unknotdb-label", {
                "prior_knot_id": prior_knot[0],
                "prior_evidence_class": prior_knot[1],
                "snappy_canonical_ids": canonical_ids,
                "census_names": row["names"],
                "cross_check": row["cross_check"],
            }))
        for reason, detail in reasons:
            db.execute("INSERT OR IGNORE INTO snappy_quarantine VALUES (?,?,?)",
                       (record["rep_key"], reason, canonical_json(detail)))
        quarantined += bool(reasons)

        # ---- a diagram that Reidemeister-reduces to nothing is the unknot,
        # ---- so any positive stored upper bound on it is not tight ---------
        if row["status"] == "unknot-diagram" and record["u_upper"]:
            db.execute(
                "INSERT OR REPLACE INTO snappy_u_upper_tightenings VALUES (?,?,?,?,?,?,?)",
                (record["rep_key"], record["node_id"], int(record["u_upper"]), 0,
                 "snappy-global-simplify-to-zero-crossings", "attested",
                 row["crossings_in"]))

        # ---- agreement with an existing UnknotDB label -------------------
        if prior_knot:
            if row["status"] == "unknot-diagram":
                observed = ["0_1"]
            else:
                observed = canonical_ids
            if not observed:
                verdict = f"snappy-silent-{row['status']}"
            elif prior_knot[0] in observed:
                verdict = "agree" if len(observed) == 1 else "agree-among-ambiguous"
            else:
                verdict = "disagree"
            db.execute(
                "INSERT OR REPLACE INTO snappy_prior_label_agreement VALUES (?,?,?,?,?,?)",
                (component, prior_knot[0], prior_knot[1], row["status"],
                 canonical_json(observed), verdict))

        # ---- pending equivalence candidates -----------------------------
        if record["rep_key"] in pending_left:
            candidate_id, candidate_knot = pending_left[record["rep_key"]]
            observed = ["0_1"] if row["status"] == "unknot-diagram" else canonical_ids
            if not observed:
                verdict = f"unresolved-{row['status']}"
            elif candidate_knot in observed and len(observed) == 1:
                verdict = "supports-candidate"
            elif candidate_knot in observed:
                verdict = "supports-candidate-among-ambiguous"
            else:
                verdict = "contradicts-candidate"
            db.execute(
                "INSERT OR REPLACE INTO snappy_candidate_resolutions VALUES (?,?,?,?,?,?)",
                (candidate_id, record["rep_key"], candidate_knot, row["status"],
                 canonical_json(observed), verdict))

        # ---- attested label into the graph map ---------------------------
        if will_label:
            if label not in known_knot_names:
                db.execute(
                    "INSERT OR IGNORE INTO knot_ids VALUES (?,?,?)",
                    (knot_id_of(label), label, "snappy-census-name-v1"))
                known_knot_names.add(label)
                new_knot_ids += 1
            evidence = evidence_for(component, label, row, label_method)
            inserted = 0
            for (rep_key,) in db.execute(
                "SELECT rep_key FROM graph_vertices WHERE cc0_component_key=?",
                (component,),
            ).fetchall():
                db.execute(
                    "INSERT OR IGNORE INTO graph_vertex_knot_map VALUES (?,?,?,?,?,?)",
                    (rep_key, knot_id_of(label), None, "attested",
                     f"attested-{label_method}-and-cc0", evidence))
                inserted += db.execute("SELECT changes()").fetchone()[0]
            labelled_components += 1
            labelled_vertices += inserted

        if time.monotonic() - last >= args.progress_seconds:
            rate = processed / max(time.monotonic() - started, 1e-9)
            remaining = (len(order) - processed) / max(rate, 1e-9)
            print(f"  {processed:,}/{len(order):,} "
                  f"t={time.monotonic()-started:.0f}s eta={remaining/60:.1f}min "
                  f"labelled={labelled_components:,} quarantine={quarantined} "
                  f"{dict(stats)}", flush=True)
            last = time.monotonic()
            db.commit()

    elapsed = time.monotonic() - started
    counts = {
        "components_considered": len(order),
        "components_processed": processed,
        "decode_failures": decode_failures,
        "previously_labelled_components": len(prior),
        "newly_labelled_components": labelled_components,
        "newly_labelled_graph_vertices": labelled_vertices,
        "new_knot_ids": new_knot_ids,
        "ambiguous_census_rows": ambiguous_rows,
        "ambiguous_fingerprint_rows": db.execute(
            "SELECT count(*) FROM snappy_quarantine "
            "WHERE reason='ambiguous-fingerprint-match'").fetchone()[0],
        "quarantined_rows": quarantined,
        "invariant_disagreements": db.execute(
            "SELECT count(*) FROM snappy_quarantine WHERE reason='invariant-disagreement'"
        ).fetchone()[0],
        "u_upper_tightenings_to_zero": db.execute(
            "SELECT count(*) FROM snappy_u_upper_tightenings").fetchone()[0],
        "connected_sum_rows": db.execute(
            "SELECT count(DISTINCT rep_key) FROM snappy_connected_sum_decompositions"
        ).fetchone()[0],
        "labelled_by_snappy_census": db.execute(
            "SELECT count(*) FROM snappy_census_identifications "
            "WHERE labelled=1 AND label_method=?", (METHOD,)).fetchone()[0],
        "labelled_by_fingerprint": db.execute(
            "SELECT count(*) FROM snappy_census_identifications "
            "WHERE labelled=1 AND label_method=?", (FINGERPRINT_METHOD,)).fetchone()[0],
        "labelled_as_unknot": db.execute(
            "SELECT count(*) FROM snappy_census_identifications WHERE labelled=1 "
            "AND label_method='snappy-global-simplify-to-zero-crossings'"
        ).fetchone()[0],
        "prior_label_disagreements": db.execute(
            "SELECT count(*) FROM snappy_prior_label_agreement WHERE verdict='disagree'"
        ).fetchone()[0],
        "mapped_graph_vertices": db.execute(
            "SELECT count(*) FROM graph_vertex_knot_map").fetchone()[0],
        "postings": db.execute(
            "SELECT count(*) FROM knot_representation_postings").fetchone()[0],
        "truncated_by_deadline": int(truncated),
    }
    report_json: dict[str, Any] = {
        "engine": engine,
        "input": str(args.input),
        "input_sha256": input_sha256,
        "counts": counts,
        "status_histogram": dict(stats),
        "cross_check_histogram": dict(cross_stats),
        "fingerprint_histogram": dict(fingerprint_stats),
        "rigor_histogram": dict(rigor_stats),
        "status_by_simplified_crossings": {
            str(k): dict(v) for k, v in sorted(by_crossings.items())},
        "seconds_per_item": total_seconds / max(processed, 1),
        "invariant_seconds_per_item": invariant_seconds / max(processed, 1),
        "reference_bundle_seconds": refs.seconds,
        "reference_bundles_cached": len(refs.cache),
        "alias_collisions_dropped": len(refs.alias_collisions),
        "elapsed_seconds": elapsed,
    }

    db.execute("UPDATE meta SET value=? WHERE key='schema'", (SCHEMA,))
    db.executemany(
        "INSERT OR REPLACE INTO meta VALUES (?,?)",
        [
            ("snappy_attestation_parent_sha256", input_sha256),
            ("snappy_attestation_engine", canonical_json(engine)),
            ("snappy_attestation_policy",
             "external-snappy-census-identification-attested-never-verified-v0"),
            ("snappy_attestation_federation_sha256", file_sha256(args.federation)),
            *((f"count_snappy_{key}", str(value)) for key, value in counts.items()),
        ],
    )
    db.execute("PRAGMA optimize")
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    foreign_keys = db.execute("PRAGMA foreign_key_check").fetchall()
    if integrity != "ok" or foreign_keys:
        raise RuntimeError(f"integrity={integrity} foreign_keys={foreign_keys[:3]}")
    db.commit()
    db.close()
    os.replace(temp, args.output)
    report_json["output"] = str(args.output)
    report_json["output_bytes"] = args.output.stat().st_size
    report_json["output_sha256"] = file_sha256(args.output)
    args.report_json.write_text(json.dumps(report_json, indent=1, sort_keys=True))
    args.report.write_text(
        "# SnapPy census attestation sidecar v6\n\n"
        f"- Input: `{args.input}` (SHA-256 `{input_sha256}`)\n"
        f"- Output: `{args.output}` ({report_json['output_bytes']:,} bytes; "
        f"SHA-256 `{report_json['output_sha256']}`)\n"
        f"- Engine: `{canonical_json(engine)}`\n"
        f"- Elapsed: {elapsed:.1f} seconds "
        f"({report_json['seconds_per_item']:.4f} s/item)\n"
        + "\n".join(f"- {key.replace('_', ' ').title()}: {value:,}"
                     for key, value in counts.items())
        + "\n- SQLite integrity and foreign keys: `ok`\n\n"
        "Every row here is an external SnapPy attestation. It improves "
        "identification coverage and search guidance. It is never `verified`, it "
        "creates no proof-graph edge, and it does not turn a catalogue "
        "unknotting number into a theorem.\n"
    )
    print(canonical_json({**counts, "elapsed_seconds": elapsed}))


if __name__ == "__main__":
    main()
