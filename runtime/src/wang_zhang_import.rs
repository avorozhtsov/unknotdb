//! Import the first, U=1 layer of the Wang--Zhang five-crossing diagram.

use crate::descending::attestation;
use crate::policy::{
    compile_preprocessing_instructions, preprocess_many_to_stopping_points,
    preprocess_to_stopping_point, PolicyLimits, PolicyOracle,
};
use crate::population::PopulationGraph;
use crate::representation::{
    verify_edge_program_with_certificate, BraidRepresentation, CheckpointedProofProgram,
    ProofInstruction, SemanticAction,
};
use crate::{RepKey, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Corpus {
    format: String,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    state_id: String,
    strands: u16,
    source_word: Vec<i16>,
    cc_position: u32,
    post_cc_word: Vec<i16>,
    zero_cc_trace: serde_json::Value,
}

#[derive(Deserialize)]
struct CubeCorpus {
    format: String,
    strands: u16,
    base_word: Vec<i16>,
    marked_positions: Vec<u32>,
    terminal_zero_cc_trace: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct WangZhangU1ImportRun {
    pub rows: usize,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub inserted_certificates: usize,
    pub collateral_improvements: usize,
    pub manifest: String,
}

#[derive(Clone, Debug)]
pub struct WangZhangCubeImportRun {
    pub states: usize,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub inserted_certificates: usize,
    pub collateral_improvements: usize,
    pub manifest: String,
}

/// Import the complete five-dimensional crossing-change cube bottom-up.
/// State bit `i` says that published marked crossing `i` has been changed.
/// Every directed edge sets one previously-clear bit, hence contains exactly
/// one CC in the common Wang--Zhang braid chart before target preprocessing.
pub fn import_wang_zhang_cube<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    corpus_bytes: &[u8],
    oracle: &mut O,
    limits: PolicyLimits,
    source_snapshot_sha256: &str,
) -> Result<WangZhangCubeImportRun> {
    let corpus: CubeCorpus = serde_json::from_slice(corpus_bytes)?;
    if corpus.format != "unknotdb-wang-zhang-five-crossing-cube-v0" {
        return Err(format!("unsupported Wang--Zhang cube `{}`", corpus.format).into());
    }
    if corpus.marked_positions.len() != 5 {
        return Err("Wang--Zhang cube must contain exactly five marked positions".into());
    }
    let base = BraidRepresentation {
        strands: corpus.strands,
        cyclic_band_generators: false,
        word: corpus.base_word.clone(),
    };
    base.validate()?;
    let mut unique = corpus.marked_positions.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != 5
        || unique
            .iter()
            .any(|position| usize::try_from(*position).map_or(true, |p| p >= base.word.len()))
    {
        return Err("Wang--Zhang marked positions are duplicate or out of range".into());
    }

    let raw_states: Vec<_> = (0_u8..32)
        .map(|mask| {
            let mut state = base.clone();
            for (bit, position) in corpus.marked_positions.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    let slot = &mut state.word[*position as usize];
                    *slot = -*slot;
                }
            }
            state
        })
        .collect();
    let reports = preprocess_many_to_stopping_points(&raw_states, oracle, limits)?;
    if reports
        .iter()
        .any(|report| !report.is_graph_stopping_point())
    {
        return Err("Wang--Zhang cube preprocessing produced an inadmissible state".into());
    }
    let programs = reports
        .iter()
        .map(|report| {
            Ok(CheckpointedProofProgram {
                instructions: compile_preprocessing_instructions(report)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let before_certificates = graph.planar_certificate_count();
    let before_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let certificate = serde_json::to_vec(&corpus.terminal_zero_cc_trace)?;
    crate::planar::verify_certificate(&raw_states[31], &certificate)?;
    let mut terminal_instructions = programs[31]
        .invert_exact(&raw_states[31])?
        .program
        .instructions;
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
    verify_edge_program_with_certificate(
        &reports[31].output.encoded,
        &unknot.encode()?,
        &terminal_program.encode()?,
        CheckpointedProofProgram::VERSION,
        terminal_program.first_action()?.encode_u63()?,
        0,
        Some(&certificate),
    )?;
    graph.relax_unknot_edge_with_planar_certificate(
        reports[31].output.clone(),
        attestation(&reports[31])?,
        graph.unknot_key()?,
        terminal_program,
        certificate,
        false,
    )?;

    let mut manifest = format!(
        "format\tunknotdb-wang-zhang-five-crossing-cube-import-v0\n\
         source_snapshot_sha256\t{source_snapshot_sha256}\n\
         corpus_sha256\t{}\npolicy_model_id\t{}\npolicy_adapter\t{}\n\
         policy_limits\t{}\t{}\nstate\tmask\tchanged\tkey\tu_upper\tstop_reason\n\
         edge\tsource_mask\ttarget_mask\tmarked_index\tsource_key\ttarget_key\n",
        unknotdb::util::sha256_hex(corpus_bytes),
        oracle.model_id(),
        crate::policy::POLICY_ADAPTER_VERSION,
        limits.max_policy_plies,
        limits.max_semantic_moves,
    );

    // Targets are already connected when their sources are inserted.
    for changed in (0_u32..=4).rev() {
        for mask in 0_u8..31 {
            if mask.count_ones() != changed {
                continue;
            }
            for bit in 0..5_usize {
                if mask & (1 << bit) != 0 {
                    continue;
                }
                let target_mask = mask | (1 << bit);
                let crossing = SemanticAction::CrossingChange {
                    position: corpus.marked_positions[bit],
                };
                if crossing.apply(&raw_states[mask as usize])? != raw_states[target_mask as usize] {
                    return Err("Wang--Zhang cube crossing does not reach target raw state".into());
                }
                let mut instructions = programs[mask as usize]
                    .invert_exact(&raw_states[mask as usize])?
                    .program
                    .instructions;
                instructions.push(ProofInstruction::Action(crossing));
                instructions.extend(programs[target_mask as usize].instructions.clone());
                let edge_program = CheckpointedProofProgram { instructions }
                    .canonicalize_semantic_first(&reports[mask as usize].output.representation)?;
                if edge_program.cc_cost() != 1
                    || edge_program.replay(&reports[mask as usize].output.representation)?
                        != reports[target_mask as usize].output.representation
                {
                    return Err("Wang--Zhang cube edge failed exact replay".into());
                }
                graph.relax_unknot_edge_deferred_acs10(
                    reports[mask as usize].output.clone(),
                    attestation(&reports[mask as usize])?,
                    reports[target_mask as usize].output.key,
                    edge_program,
                    None,
                )?;
                manifest.push_str(&format!(
                    "edge\t{mask:02x}\t{target_mask:02x}\t{bit}\t{}\t{}\n",
                    hex(&reports[mask as usize].output.key),
                    hex(&reports[target_mask as usize].output.key),
                ));
            }
        }
        graph.recompute_unknot_routes()?;
    }
    graph.recompute_unknot_routes()?;
    graph.recompute_acs10_routes()?;
    for mask in 0_u8..32 {
        let u = graph
            .u_upper_bound(&reports[mask as usize].output.key)
            .ok_or("Wang--Zhang cube state has no finite U route")?;
        let expected = 5 - mask.count_ones();
        if u > expected {
            return Err(
                format!("cube mask {mask:02x} has U={u}, expected at most {expected}").into(),
            );
        }
        manifest.push_str(&format!(
            "state\t{mask:02x}\t{}\t{}\t{u}\t{:?}\n",
            mask.count_ones(),
            hex(&reports[mask as usize].output.key),
            reports[mask as usize].stop_reason,
        ));
    }
    let after_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let collateral_improvements = before_u
        .iter()
        .filter(|(key, old)| after_u.get(*key).is_some_and(|new| new < *old))
        .count();
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    let inserted_certificates = graph.planar_certificate_count() - before_certificates;
    manifest.push_str(&format!(
        "summary\tstates\t32\nsummary\tinserted_nodes\t{inserted_nodes}\n\
         summary\tinserted_edges\t{inserted_edges}\n\
         summary\tinserted_certificates\t{inserted_certificates}\n\
         summary\tcollateral_improvements\t{collateral_improvements}\n"
    ));
    Ok(WangZhangCubeImportRun {
        states: 32,
        inserted_nodes,
        inserted_edges,
        inserted_certificates,
        collateral_improvements,
        manifest,
    })
}

pub fn import_wang_zhang_u1<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    corpus_bytes: &[u8],
    oracle: &mut O,
    limits: PolicyLimits,
    source_snapshot_sha256: &str,
) -> Result<WangZhangU1ImportRun> {
    let corpus: Corpus = serde_json::from_slice(corpus_bytes)?;
    if corpus.format != "unknotdb-wang-zhang-u1-layer-v0" {
        return Err(format!("unsupported Wang--Zhang corpus `{}`", corpus.format).into());
    }
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let before_certificates = graph.planar_certificate_count();
    let before_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let mut manifest = format!(
        "format\tunknotdb-wang-zhang-u1-import-v0\n\
         source_snapshot_sha256\t{source_snapshot_sha256}\n\
         corpus_sha256\t{}\n\
         policy_model_id\t{}\n\
         policy_limits\t{}\t{}\n\
         row\tstate_id\tsource_key\tsuccessor_key\tcertificate_id\tnew_u\n",
        unknotdb::util::sha256_hex(corpus_bytes),
        oracle.model_id(),
        limits.max_policy_plies,
        limits.max_semantic_moves,
    );
    for row in &corpus.rows {
        let source_raw = BraidRepresentation {
            strands: row.strands,
            cyclic_band_generators: false,
            word: row.source_word.clone(),
        };
        let crossing = SemanticAction::CrossingChange {
            position: row.cc_position,
        };
        let successor_raw = crossing.apply(&source_raw)?;
        if successor_raw.word != row.post_cc_word {
            return Err(format!("{} post-CC word mismatch", row.state_id).into());
        }
        let source_report = preprocess_to_stopping_point(&source_raw, oracle, limits)?;
        let successor_report = preprocess_to_stopping_point(&successor_raw, oracle, limits)?;
        if !source_report.is_graph_stopping_point() || !successor_report.is_graph_stopping_point() {
            return Err(format!("{} preprocessing did not reach graph stops", row.state_id).into());
        }
        let certificate = serde_json::to_vec(&row.zero_cc_trace)?;
        crate::planar::verify_certificate(&successor_raw, &certificate)?;

        let successor_forward = CheckpointedProofProgram {
            instructions: compile_preprocessing_instructions(&successor_report)?,
        };
        let mut terminal_instructions = if successor_forward.instructions.is_empty() {
            Vec::new()
        } else {
            successor_forward
                .invert_exact(&successor_raw)?
                .program
                .instructions
        };
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
        verify_edge_program_with_certificate(
            &successor_report.output.encoded,
            &unknot.encode()?,
            &terminal_program.encode()?,
            CheckpointedProofProgram::VERSION,
            terminal_program.first_action()?.encode_u63()?,
            0,
            Some(&certificate),
        )?;
        let certificate_id = unknotdb::util::sha256(&certificate);
        let unknot_key = graph.unknot_key()?;
        graph.relax_unknot_edge_with_planar_certificate(
            successor_report.output.clone(),
            attestation(&successor_report)?,
            unknot_key,
            terminal_program,
            certificate,
            false,
        )?;

        let source_forward = CheckpointedProofProgram {
            instructions: compile_preprocessing_instructions(&source_report)?,
        };
        let mut crossing_instructions = if source_forward.instructions.is_empty() {
            Vec::new()
        } else {
            source_forward
                .invert_exact(&source_raw)?
                .program
                .instructions
        };
        crossing_instructions.push(ProofInstruction::Action(crossing));
        crossing_instructions.extend(successor_forward.instructions);
        let crossing_program = CheckpointedProofProgram {
            instructions: crossing_instructions,
        }
        .canonicalize_semantic_first(&source_report.output.representation)?;
        if crossing_program.cc_cost() != 1
            || crossing_program.replay(&source_report.output.representation)?
                != successor_report.output.representation
        {
            return Err(format!("{} one-CC edge failed exact replay", row.state_id).into());
        }
        graph.relax_unknot_edge_deferred_acs10(
            source_report.output.clone(),
            attestation(&source_report)?,
            successor_report.output.key,
            crossing_program,
            None,
        )?;
        graph.recompute_unknot_routes()?;
        let new_u = graph
            .u_upper_bound(&source_report.output.key)
            .ok_or("Wang--Zhang source disappeared")?;
        if new_u != 1 {
            return Err(format!("{} expected U=1, got {new_u}", row.state_id).into());
        }
        manifest.push_str(&format!(
            "row\t{}\t{}\t{}\t{}\t{}\n",
            row.state_id,
            hex(&source_report.output.key),
            hex(&successor_report.output.key),
            hex(&certificate_id),
            new_u,
        ));
    }
    graph.recompute_unknot_routes()?;
    graph.recompute_acs10_routes()?;
    let after_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let collateral_improvements = before_u
        .iter()
        .filter(|(key, old)| after_u.get(*key).is_some_and(|new| new < *old))
        .count();
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    let inserted_certificates = graph.planar_certificate_count() - before_certificates;
    manifest.push_str(&format!(
        "summary\trows\t{}\nsummary\tinserted_nodes\t{inserted_nodes}\n\
         summary\tinserted_edges\t{inserted_edges}\n\
         summary\tinserted_certificates\t{inserted_certificates}\n\
         summary\tcollateral_improvements\t{collateral_improvements}\n",
        corpus.rows.len(),
    ));
    Ok(WangZhangU1ImportRun {
        rows: corpus.rows.len(),
        inserted_nodes,
        inserted_edges,
        inserted_certificates,
        collateral_improvements,
        manifest,
    })
}

fn hex(bytes: &RepKey) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
