//! Import exact catalogue crossing-change sets backed by planar RI/RII/RIII traces.

use crate::descending::{attestation, ChainEdge};
use crate::policy::{
    compile_preprocessing_instructions, preprocess_to_stopping_point, PolicyLimits, PolicyOracle,
};
use crate::population::PopulationGraph;
use crate::representation::{
    verify_edge_program_with_certificate, BraidRepresentation, CheckpointedProofProgram,
    ProofInstruction, SemanticAction,
};
use crate::{PolicyStopAttestation, RepKey, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct CatalogueFile {
    results: Vec<CatalogueResult>,
}

#[derive(Deserialize)]
struct CatalogueResult {
    knot_id: String,
    catalogue_exact_u: u32,
    standard_braid: StandardBraid,
    search: SearchResult,
    witness: Option<CatalogueWitness>,
}

#[derive(Deserialize)]
struct StandardBraid {
    strands: u16,
    normalized_word: Vec<i16>,
    rep_key: String,
}

#[derive(Deserialize)]
struct SearchResult {
    status: String,
}

#[derive(Deserialize)]
struct CatalogueWitness {
    crossing_change_positions: Vec<u32>,
    post_cc_word: Vec<i16>,
    zero_cc_trace: serde_json::Value,
}

#[derive(Clone)]
struct CompiledWitness {
    source_key: RepKey,
    source_was_absent: bool,
    edges: Vec<ChainEdge>,
    terminal_source: crate::representation::NormalizedRepresentation,
    terminal_stop: PolicyStopAttestation,
    terminal_program: CheckpointedProofProgram,
    certificate: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct CataloguePlanarImportRun {
    pub corpus_entries: usize,
    pub witness_entries: usize,
    pub strict_candidates: usize,
    pub improved: usize,
    pub already_sufficient: usize,
    pub absent_sources: usize,
    pub newly_connected_sources: usize,
    pub failed: usize,
    pub direct_improvements: usize,
    pub collateral_improvements: usize,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub inserted_certificates: usize,
    pub manifest: String,
}

pub fn import_catalogue_planar<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    corpus_bytes: &[(&str, &[u8])],
    oracle: &mut O,
    policy_limits: PolicyLimits,
    source_snapshot_sha256: &str,
) -> Result<CataloguePlanarImportRun> {
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let before_certificates = graph.planar_certificate_count();
    let before_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let mut manifest = format!(
        "format\tunknotdb-catalogue-planar-chain-import-v0\n\
         source_snapshot_sha256\t{source_snapshot_sha256}\n\
         policy_model_id\t{}\n\
         max_policy_plies\t{}\n\
         max_semantic_moves\t{}\n\
         row\tknot_id\tstatus\tsource_key\told_u\tnew_u\tcc_count\tcertificate_id\tdetail\n",
        oracle.model_id(),
        policy_limits.max_policy_plies,
        policy_limits.max_semantic_moves,
    );
    let mut corpus_entries = 0_usize;
    let mut witness_entries = 0_usize;
    let mut strict_candidates = 0_usize;
    let mut improved = 0_usize;
    let mut already_sufficient = 0_usize;
    let mut absent_sources = 0_usize;
    let mut newly_connected_sources = 0_usize;
    let mut failed = 0_usize;

    for (source_name, bytes) in corpus_bytes {
        let corpus: CatalogueFile = serde_json::from_slice(bytes)?;
        manifest.push_str(&format!(
            "corpus\t{}\t{}\t{}\n",
            source_name,
            unknotdb::util::sha256_hex(bytes),
            corpus.results.len()
        ));
        corpus_entries += corpus.results.len();
        for entry in corpus.results {
            if entry.search.status != "witness" || entry.witness.is_none() {
                continue;
            }
            witness_entries += 1;
            let source_key = match parse_key(&entry.standard_braid.rep_key) {
                Ok(key) => key,
                Err(error) => {
                    failed += 1;
                    push_row(
                        &mut manifest,
                        &entry.knot_id,
                        "failed",
                        None,
                        None,
                        0,
                        None,
                        None,
                        &error.to_string(),
                    );
                    continue;
                }
            };
            let raw_old_u = graph.u_upper_bound(&source_key);
            if raw_old_u.is_none() {
                absent_sources += 1;
            }
            if raw_old_u.is_some_and(|old_u| old_u <= entry.catalogue_exact_u) {
                already_sufficient += 1;
                push_row(
                    &mut manifest,
                    &entry.knot_id,
                    "already-sufficient",
                    raw_old_u,
                    raw_old_u,
                    entry.catalogue_exact_u,
                    None,
                    Some(source_key),
                    "graph bound is already no larger than witness cost",
                );
                continue;
            }
            let witness = entry.witness.as_ref().unwrap();
            match compile_witness(graph, &entry, witness, oracle, policy_limits) {
                Ok(compiled) => {
                    let old_u = graph.u_upper_bound(&compiled.source_key);
                    if old_u.is_some_and(|bound| bound <= entry.catalogue_exact_u) {
                        already_sufficient += 1;
                        push_row(
                            &mut manifest,
                            &entry.knot_id,
                            "already-sufficient-after-preprocessing",
                            old_u,
                            old_u,
                            entry.catalogue_exact_u,
                            None,
                            Some(compiled.source_key),
                            "preprocessed stopping point already has a sufficient route",
                        );
                        continue;
                    }
                    strict_candidates += 1;
                    let certificate_id = unknotdb::util::sha256(&compiled.certificate);
                    let unknot_key = graph.unknot_key()?;
                    graph.relax_unknot_edge_with_planar_certificate(
                        compiled.terminal_source,
                        compiled.terminal_stop,
                        unknot_key,
                        compiled.terminal_program,
                        compiled.certificate,
                        false,
                    )?;
                    for edge in compiled.edges.into_iter().rev() {
                        graph.relax_unknot_edge_deferred_acs10(
                            edge.source,
                            edge.source_stop,
                            edge.target_key,
                            edge.program,
                            None,
                        )?;
                    }
                    let new_u = graph
                        .u_upper_bound(&compiled.source_key)
                        .ok_or("catalogue source disappeared after relaxation")?;
                    if old_u.is_some_and(|bound| new_u >= bound) || new_u > entry.catalogue_exact_u
                    {
                        return Err(format!(
                            "{} accepted chain did not produce its strict bound: {:?} -> {new_u}",
                            entry.knot_id, old_u,
                        )
                        .into());
                    }
                    if compiled.source_was_absent {
                        newly_connected_sources += 1;
                    }
                    improved += 1;
                    push_row(
                        &mut manifest,
                        &entry.knot_id,
                        "improved",
                        old_u,
                        Some(new_u),
                        witness.crossing_change_positions.len() as u32,
                        Some(certificate_id),
                        Some(compiled.source_key),
                        "verified chain",
                    );
                }
                Err(error) => {
                    failed += 1;
                    push_row(
                        &mut manifest,
                        &entry.knot_id,
                        "failed",
                        raw_old_u,
                        raw_old_u,
                        witness.crossing_change_positions.len() as u32,
                        None,
                        Some(source_key),
                        &error.to_string(),
                    );
                }
            }
        }
    }
    graph.recompute_unknot_routes()?;
    graph.recompute_acs10_routes()?;
    let after_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let all_improvements = before_u
        .iter()
        .filter(|(key, old)| after_u.get(*key).is_some_and(|new| new < *old))
        .count();
    let collateral_improvements = all_improvements.saturating_sub(improved);
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    let inserted_certificates = graph.planar_certificate_count() - before_certificates;
    manifest.push_str(&format!(
        "summary\tcorpus_entries\t{corpus_entries}\n\
         summary\twitness_entries\t{witness_entries}\n\
         summary\tstrict_candidates\t{strict_candidates}\n\
         summary\timproved\t{improved}\n\
         summary\talready_sufficient\t{already_sufficient}\n\
         summary\tabsent_sources\t{absent_sources}\n\
         summary\tnewly_connected_sources\t{newly_connected_sources}\n\
         summary\tfailed\t{failed}\n\
         summary\tcollateral_improvements\t{collateral_improvements}\n\
         summary\tinserted_nodes\t{inserted_nodes}\n\
         summary\tinserted_edges\t{inserted_edges}\n\
         summary\tinserted_certificates\t{inserted_certificates}\n"
    ));
    Ok(CataloguePlanarImportRun {
        corpus_entries,
        witness_entries,
        strict_candidates,
        improved,
        already_sufficient,
        absent_sources,
        newly_connected_sources,
        failed,
        direct_improvements: improved,
        collateral_improvements,
        inserted_nodes,
        inserted_edges,
        inserted_certificates,
        manifest,
    })
}

fn compile_witness<O: PolicyOracle>(
    graph: &PopulationGraph,
    entry: &CatalogueResult,
    witness: &CatalogueWitness,
    oracle: &mut O,
    policy_limits: PolicyLimits,
) -> Result<CompiledWitness> {
    if witness.crossing_change_positions.len() as u32 != entry.catalogue_exact_u {
        return Err("witness CC count differs from catalogue cost".into());
    }
    let source_key = parse_key(&entry.standard_braid.rep_key)?;
    let raw_source = BraidRepresentation {
        strands: entry.standard_braid.strands,
        cyclic_band_generators: false,
        word: entry.standard_braid.normalized_word.clone(),
    };
    let normalized_source = raw_source.normalize()?;
    if normalized_source.key != source_key || normalized_source.representation != raw_source {
        return Err("catalogue standard braid does not match graph source encoding".into());
    }
    let (mut current, mut current_stop, mut previous_preprocessing, source_was_absent) =
        if let Some(seed) = graph.expansion_seed(&source_key)? {
            (seed.representation, seed.policy_stop, None, false)
        } else {
            let report = preprocess_to_stopping_point(&raw_source, oracle, policy_limits)?;
            if !report.is_graph_stopping_point() {
                return Err(format!(
                    "catalogue source preprocessing stopped at {:?}",
                    report.stop_reason
                )
                .into());
            }
            let preprocessing = CheckpointedProofProgram {
                instructions: compile_preprocessing_instructions(&report)?,
            };
            let source_was_absent = graph.u_upper_bound(&report.output.key).is_none();
            (
                report.output.clone(),
                attestation(&report)?,
                Some((raw_source.clone(), preprocessing)),
                source_was_absent,
            )
        };
    let effective_source_key = current.key;
    let mut raw = raw_source;
    let mut edges = Vec::new();
    for &position in &witness.crossing_change_positions {
        let mut instructions = inverse_preprocessing(&previous_preprocessing)?;
        let crossing = SemanticAction::CrossingChange { position };
        instructions.push(ProofInstruction::Action(crossing));
        raw = crossing.apply(&raw)?;
        let report = preprocess_to_stopping_point(&raw, oracle, policy_limits)?;
        if !report.is_graph_stopping_point() {
            return Err(
                format!("post-CC preprocessing stopped at {:?}", report.stop_reason).into(),
            );
        }
        let preprocessing = CheckpointedProofProgram {
            instructions: compile_preprocessing_instructions(&report)?,
        };
        instructions.extend(preprocessing.instructions.iter().copied());
        let program = CheckpointedProofProgram { instructions }
            .canonicalize_semantic_first(&current.representation)?;
        if program.cc_cost() != 1
            || program.replay(&current.representation)? != report.output.representation
        {
            return Err("catalogue one-CC chain edge failed exact replay".into());
        }
        edges.push(ChainEdge {
            source: current,
            source_stop: current_stop,
            target_key: report.output.key,
            program,
        });
        current = report.output.clone();
        current_stop = attestation(&report)?;
        previous_preprocessing = Some((raw.clone(), preprocessing));
    }
    if raw.word != witness.post_cc_word {
        return Err("sequential CC replay does not match witnessed post-CC word".into());
    }
    let certificate = serde_json::to_vec(&witness.zero_cc_trace)?;
    crate::planar::verify_certificate(&raw, &certificate)?;
    let mut terminal_instructions = inverse_preprocessing(&previous_preprocessing)?;
    if terminal_instructions
        .iter()
        .any(|instruction| matches!(instruction, ProofInstruction::Action(_)))
    {
        terminal_instructions = CheckpointedProofProgram {
            instructions: terminal_instructions,
        }
        .canonicalize_semantic_first(&current.representation)?
        .instructions;
    }
    terminal_instructions.push(ProofInstruction::Action(
        SemanticAction::PlanarCertificateCollapse,
    ));
    let terminal_program = CheckpointedProofProgram {
        instructions: terminal_instructions,
    };
    let unknot = BraidRepresentation {
        strands: 1,
        cyclic_band_generators: false,
        word: Vec::new(),
    };
    let program_bytes = terminal_program.encode()?;
    verify_edge_program_with_certificate(
        &current.encoded,
        &unknot.encode()?,
        &program_bytes,
        CheckpointedProofProgram::VERSION,
        terminal_program.first_action()?.encode_u63()?,
        0,
        Some(&certificate),
    )?;
    Ok(CompiledWitness {
        source_key: effective_source_key,
        source_was_absent,
        edges,
        terminal_source: current,
        terminal_stop: current_stop,
        terminal_program,
        certificate,
    })
}

fn inverse_preprocessing(
    previous: &Option<(BraidRepresentation, CheckpointedProofProgram)>,
) -> Result<Vec<ProofInstruction>> {
    let Some((raw, forward)) = previous else {
        return Ok(Vec::new());
    };
    if forward.instructions.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(forward.invert_exact(raw)?.program.instructions)
    }
}

fn parse_key(value: &str) -> Result<RepKey> {
    if value.len() != 64 {
        return Err("representation key is not 64 hexadecimal digits".into());
    }
    let mut key = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        key[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(key)
}

#[allow(clippy::too_many_arguments)]
fn push_row(
    manifest: &mut String,
    knot: &str,
    status: &str,
    old_u: Option<u32>,
    new_u: Option<u32>,
    cc_count: u32,
    certificate_id: Option<RepKey>,
    source_key: Option<RepKey>,
    detail: &str,
) {
    let clean = detail.replace(['\t', '\n', '\r'], " ");
    manifest.push_str(&format!(
        "result\t{knot}\t{status}\t{}\t{}\t{}\t{cc_count}\t{}\t{clean}\n",
        source_key.map_or_else(|| "-".to_owned(), |value| hex(&value)),
        old_u.map_or_else(|| "-".to_owned(), |value| value.to_string()),
        new_u.map_or_else(|| "-".to_owned(), |value| value.to_string()),
        certificate_id.map_or_else(|| "-".to_owned(), |value| hex(&value)),
    ));
}

fn hex(bytes: &RepKey) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
