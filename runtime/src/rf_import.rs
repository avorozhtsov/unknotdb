//! Import replay-verified RF best traces before using descending fallback.

use crate::campaign::RfCorpusEntry;
use crate::descending::{attestation, compile_chain, ChainEdge};
use crate::policy::{
    compile_preprocessing_instructions, preprocess_to_stopping_point, PolicyLimits, PolicyOracle,
};
use crate::population::{ExpansionSeed, PopulationGraph};
use crate::representation::{
    BraidRepresentation, CheckpointedProofProgram, ProofInstruction, SemanticAction,
};
use crate::{RepKey, Result};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

pub const RF_BEST_IMPORT_VERSION: &str = "unknotdb-rf-best-trace-import-v0";
pub const RF_DESCENDING_IMPORT_VERSION: &str = "unknotdb-rf-descending-import-v0";
const BEST_STEP_SCHEMA: &str = "unknotdb-rf-best-witness-steps-v0";

#[derive(Clone, Debug)]
struct BestStep {
    action: SemanticAction,
    after: BraidRepresentation,
}

#[derive(Clone, Debug)]
pub struct BestTrace {
    representation_id: String,
    evidence_id: String,
    pool_sha256: String,
    l1000: u32,
    l10: u32,
    declared_cc: u32,
    declared_moves: u32,
    start: BraidRepresentation,
    steps: Vec<BestStep>,
}

impl BestTrace {
    pub fn representation_id(&self) -> &str {
        &self.representation_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RfImportRun {
    pub selected: usize,
    pub initially_covered: usize,
    pub imported: usize,
    pub improved: usize,
    pub failed: usize,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub manifest: String,
}

pub fn read_best_trace_tsv(path: &Path) -> Result<Vec<BestTrace>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    let header = "schema\tpool_sha256\trepresentation_id\tevidence_id\tl1000\tl10\tdeclared_cc\tdeclared_moves\tstep_index\tstart_strands\tstart_word\tkind\tposition\tgenerator\tsign\tafter_strands\tafter_word";
    if lines.next() != Some(header) {
        return Err("RF best-witness TSV has an unsupported header".into());
    }
    let mut grouped: BTreeMap<String, BestTrace> = BTreeMap::new();
    for (line_index, line) in lines.enumerate() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 17 || fields[0] != BEST_STEP_SCHEMA {
            return Err(format!("RF best-witness line {} is malformed", line_index + 2).into());
        }
        let representation_id = fields[2].to_owned();
        let start = BraidRepresentation {
            strands: fields[9].parse()?,
            cyclic_band_generators: false,
            word: parse_word(fields[10])?,
        };
        let after = BraidRepresentation {
            strands: fields[15].parse()?,
            cyclic_band_generators: false,
            word: parse_word(fields[16])?,
        };
        start.validate()?;
        after.validate()?;
        let step_index: usize = fields[8].parse()?;
        let trace = grouped
            .entry(representation_id.clone())
            .or_insert_with(|| BestTrace {
                representation_id,
                evidence_id: fields[3].to_owned(),
                pool_sha256: fields[1].to_owned(),
                l1000: fields[4].parse().unwrap_or(u32::MAX),
                l10: fields[5].parse().unwrap_or(u32::MAX),
                declared_cc: fields[6].parse().unwrap_or(u32::MAX),
                declared_moves: fields[7].parse().unwrap_or(u32::MAX),
                start: start.clone(),
                steps: Vec::new(),
            });
        if trace.start != start
            || trace.evidence_id != fields[3]
            || trace.pool_sha256 != fields[1]
            || trace.steps.len() != step_index
        {
            return Err(format!(
                "RF best-witness line {} breaks trace grouping",
                line_index + 2
            )
            .into());
        }
        trace.steps.push(BestStep {
            action: parse_action(fields[11], fields[12], fields[13], fields[14])?,
            after,
        });
    }
    Ok(grouped.into_values().collect())
}

pub fn import_best_traces<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    corpus: &[RfCorpusEntry],
    traces: &[BestTrace],
    oracle: &mut O,
    snapshot_id: &str,
    policy_limits: PolicyLimits,
) -> Result<RfImportRun> {
    let corpus_ids: HashSet<_> = corpus
        .iter()
        .map(|row| row.representation_id.as_str())
        .collect();
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let mut manifest = format!(
        "{RF_BEST_IMPORT_VERSION}\nsnapshot_id\t{snapshot_id}\nmodel_id\t{}\npolicy_limits\t{}\t{}\n",
        oracle.model_id(), policy_limits.max_policy_plies, policy_limits.max_semantic_moves,
    );
    manifest.push_str(
        "trace\trepresentation_id\tevidence_id\tl1000\tl10\told_u\tnew_u\tedges\tstatus\n",
    );
    let mut initially_covered = 0;
    let mut imported = 0;
    let mut improved = 0;
    let mut failed = 0;
    for trace in traces {
        if !corpus_ids.contains(trace.representation_id.as_str()) {
            failed += 1;
            manifest.push_str(&format!(
                "trace\t{}\t{}\t{}\t{}\t-\t-\t0\tnot-in-corpus\n",
                trace.representation_id, trace.evidence_id, trace.l1000, trace.l10,
            ));
            continue;
        }
        let initial = preprocess_to_stopping_point(&trace.start, oracle, policy_limits)?;
        if !initial.is_graph_stopping_point() {
            failed += 1;
            manifest.push_str(&format!(
                "trace\t{}\t{}\t{}\t{}\t-\t-\t0\tpreprocess:{:?}\n",
                trace.representation_id,
                trace.evidence_id,
                trace.l1000,
                trace.l10,
                initial.stop_reason,
            ));
            continue;
        }
        let old_u = graph.u_upper_bound(&initial.output.key);
        initially_covered += usize::from(old_u.is_some());
        match compile_best_trace(trace, &initial, oracle, policy_limits) {
            Ok(edges) => {
                let mut staged = graph.clone();
                for edge in edges.iter().rev() {
                    staged.relax_unknot_edge_deferred_acs10(
                        edge.source.clone(),
                        edge.source_stop,
                        edge.target_key,
                        edge.program.clone(),
                        Some(unknotdb::util::sha256(trace.evidence_id.as_bytes())),
                    )?;
                }
                staged.recompute_unknot_routes()?;
                staged.recompute_acs10_routes()?;
                let new_u = staged
                    .u_upper_bound(&initial.output.key)
                    .ok_or("RF best trace did not connect its stopping point")?;
                improved += usize::from(old_u.is_some_and(|old| new_u < old));
                imported += 1;
                manifest.push_str(&format!(
                    "trace\t{}\t{}\t{}\t{}\t{}\t{}\t{}\tverified\n",
                    trace.representation_id,
                    trace.evidence_id,
                    trace.l1000,
                    trace.l10,
                    optional_u(old_u),
                    new_u,
                    edges.len(),
                ));
                *graph = staged;
            }
            Err(error) => {
                failed += 1;
                manifest.push_str(&format!(
                    "trace\t{}\t{}\t{}\t{}\t{}\t-\t0\tfailed:{}\n",
                    trace.representation_id,
                    trace.evidence_id,
                    trace.l1000,
                    trace.l10,
                    optional_u(old_u),
                    sanitize(&error.to_string()),
                ));
            }
        }
    }
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\n",
        traces.len(),
        initially_covered,
        imported,
        improved,
        failed,
        inserted_edges,
    ));
    Ok(RfImportRun {
        selected: traces.len(),
        initially_covered,
        imported,
        improved,
        failed,
        inserted_nodes,
        inserted_edges,
        manifest,
    })
}

pub fn import_descending_corpus<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    corpus: &[RfCorpusEntry],
    oracle: &mut O,
    snapshot_id: &str,
    policy_limits: PolicyLimits,
    skip: usize,
    limit: usize,
) -> Result<RfImportRun> {
    let selected: Vec<_> = corpus.iter().skip(skip).take(limit).collect();
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let mut manifest = format!(
        "{RF_DESCENDING_IMPORT_VERSION}\nsnapshot_id\t{snapshot_id}\nmodel_id\t{}\nrange\t{}\t{}\npolicy_limits\t{}\t{}\n",
        oracle.model_id(), skip, selected.len(), policy_limits.max_policy_plies,
        policy_limits.max_semantic_moves,
    );
    manifest.push_str("representation\tid\tstopping_key\told_u\tnew_u\tcc\tedges\tstatus\n");
    let mut initially_covered = 0;
    let mut imported = 0;
    let mut improved = 0;
    let mut failed = 0;
    for entry in &selected {
        let report =
            match preprocess_to_stopping_point(&entry.representation, oracle, policy_limits) {
                Ok(report) => report,
                Err(error) => {
                    failed += 1;
                    manifest.push_str(&format!(
                        "representation\t{}\t-\t-\t-\t-\t0\tfailed:{}\n",
                        entry.representation_id,
                        sanitize(&error.to_string()),
                    ));
                    continue;
                }
            };
        if !report.is_graph_stopping_point() {
            failed += 1;
            manifest.push_str(&format!(
                "representation\t{}\t-\t-\t-\t-\t0\tpreprocess:{:?}\n",
                entry.representation_id, report.stop_reason,
            ));
            continue;
        }
        let old_u = graph.u_upper_bound(&report.output.key);
        if let Some(value) = old_u {
            initially_covered += 1;
            manifest.push_str(&format!(
                "representation\t{}\t{}\t{}\t{}\t0\t0\talready-covered\n",
                entry.representation_id,
                hex(&report.output.key),
                value,
                value,
            ));
            continue;
        }
        let seed = ExpansionSeed {
            representation: report.output.clone(),
            policy_stop: attestation(&report)?,
            u_upper_bound: u32::MAX,
            route_rank: u64::MAX,
            active_witness_semantic_len: 0,
        };
        match compile_chain(&seed, oracle, policy_limits) {
            Ok((edges, cc)) => {
                let mut staged = graph.clone();
                for edge in edges.iter().rev() {
                    staged.relax_unknot_edge_deferred_acs10(
                        edge.source.clone(),
                        edge.source_stop,
                        edge.target_key,
                        edge.program.clone(),
                        None,
                    )?;
                }
                staged.recompute_unknot_routes()?;
                staged.recompute_acs10_routes()?;
                let new_u = staged
                    .u_upper_bound(&report.output.key)
                    .ok_or("RF descending fallback did not connect its stopping point")?;
                imported += 1;
                improved += usize::from(old_u.is_some_and(|old| new_u < old));
                manifest.push_str(&format!(
                    "representation\t{}\t{}\t{}\t{}\t{}\t{}\tverified\n",
                    entry.representation_id,
                    hex(&report.output.key),
                    optional_u(old_u),
                    new_u,
                    cc,
                    edges.len(),
                ));
                *graph = staged;
            }
            Err(error) => {
                failed += 1;
                manifest.push_str(&format!(
                    "representation\t{}\t{}\t-\t-\t-\t0\tfailed:{}\n",
                    entry.representation_id,
                    hex(&report.output.key),
                    sanitize(&error.to_string()),
                ));
            }
        }
    }
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\n",
        selected.len(),
        initially_covered,
        imported,
        improved,
        failed,
        inserted_edges,
    ));
    Ok(RfImportRun {
        selected: selected.len(),
        initially_covered,
        imported,
        improved,
        failed,
        inserted_nodes,
        inserted_edges,
        manifest,
    })
}

fn compile_best_trace<O: PolicyOracle>(
    trace: &BestTrace,
    initial: &crate::policy::PreprocessingReport,
    oracle: &mut O,
    policy_limits: PolicyLimits,
) -> Result<Vec<ChainEdge>> {
    if trace.steps.len() != trace.declared_moves as usize {
        return Err("RF best trace move count differs from declaration".into());
    }
    let mut replay = trace.start.clone();
    let mut cc = 0_u32;
    for step in &trace.steps {
        replay = step.action.apply(&replay)?;
        if replay != step.after {
            return Err("RF best trace checkpoint mismatch".into());
        }
        cc += u32::from(step.action.is_crossing_change());
    }
    if cc != trace.declared_cc {
        return Err("RF best trace CC count differs from declaration".into());
    }
    let unknot = BraidRepresentation {
        strands: 1,
        cyclic_band_generators: false,
        word: Vec::new(),
    }
    .normalize()?;
    if replay.normalize()?.key != unknot.key {
        return Err("RF best trace does not terminate at the unknot".into());
    }

    let mut raw_boundary = trace.start.clone();
    let mut preprocessing = preprocessing_program(initial)?;
    let mut current = initial.output.clone();
    let mut current_stop = attestation(initial)?;
    let mut pending_zero = Vec::<SemanticAction>::new();
    let mut edges = Vec::new();
    for step in &trace.steps {
        if !step.action.is_crossing_change() {
            pending_zero.push(step.action);
            continue;
        }
        let mut instructions = inverse_program(&raw_boundary, &preprocessing)?;
        instructions.extend(pending_zero.drain(..).map(ProofInstruction::Action));
        instructions.push(ProofInstruction::Action(step.action));
        let raw_after_cc = step.after.clone();
        let report = preprocess_to_stopping_point(&raw_after_cc, oracle, policy_limits)?;
        if !report.is_graph_stopping_point() {
            return Err(format!(
                "RF post-CC preprocessing stopped at {:?}",
                report.stop_reason
            )
            .into());
        }
        let next_preprocessing = preprocessing_program(&report)?;
        instructions.extend(next_preprocessing.instructions.iter().copied());
        let program = CheckpointedProofProgram { instructions }
            .canonicalize_semantic_first(&current.representation)?;
        if program.replay(&current.representation)? != report.output.representation {
            return Err("RF best edge failed exact replay".into());
        }
        edges.push(ChainEdge {
            source: current,
            source_stop: current_stop,
            target_key: report.output.key,
            program,
        });
        current = report.output.clone();
        current_stop = attestation(&report)?;
        raw_boundary = raw_after_cc;
        preprocessing = next_preprocessing;
    }
    if current.key != unknot.key {
        let mut instructions = inverse_program(&raw_boundary, &preprocessing)?;
        instructions.extend(pending_zero.into_iter().map(ProofInstruction::Action));
        let program = CheckpointedProofProgram { instructions }
            .canonicalize_semantic_first(&current.representation)?;
        if program.replay(&current.representation)?.normalize()?.key != unknot.key {
            return Err("RF best terminal suffix failed exact replay".into());
        }
        edges.push(ChainEdge {
            source: current,
            source_stop: current_stop,
            target_key: unknot.key,
            program,
        });
    }
    Ok(edges)
}

fn preprocessing_program(
    report: &crate::policy::PreprocessingReport,
) -> Result<CheckpointedProofProgram> {
    Ok(CheckpointedProofProgram {
        instructions: compile_preprocessing_instructions(report)?,
    })
}

fn inverse_program(
    raw: &BraidRepresentation,
    forward: &CheckpointedProofProgram,
) -> Result<Vec<ProofInstruction>> {
    if forward.instructions.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(forward.invert_exact(raw)?.program.instructions)
    }
}

fn parse_action(kind: &str, position: &str, generator: &str, sign: &str) -> Result<SemanticAction> {
    let position = || -> Result<u32> { Ok(position.parse()?) };
    Ok(match kind {
        "REDUCE" => SemanticAction::Reduce {
            position: position()?,
        },
        "COMMUTE" => SemanticAction::Commute {
            position: position()?,
        },
        "BRAID" => SemanticAction::Braid {
            position: position()?,
        },
        "INSERT" => SemanticAction::Insert {
            position: position()?,
            generator: generator.parse()?,
            sign: sign.parse()?,
        },
        "DESTABILIZE" => SemanticAction::Destabilize,
        "STABILIZE_POS" => SemanticAction::StabilizePositive,
        "STABILIZE_NEG" => SemanticAction::StabilizeNegative,
        "PASS" => SemanticAction::Pass,
        "CROSSING_CHANGE" => SemanticAction::CrossingChange {
            position: position()?,
        },
        _ => return Err(format!("unsupported RF best action {kind}").into()),
    })
}

fn parse_word(value: &str) -> Result<Vec<i16>> {
    if value.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(value
            .split(',')
            .map(str::parse::<i16>)
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

fn optional_u(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".into())
}

fn hex(bytes: &RepKey) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sanitize(value: &str) -> String {
    value.replace(['\t', '\n'], " ")
}
