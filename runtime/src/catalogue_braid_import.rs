//! Promote diagram-attested DT claims that braidify to an exact one-CC macro.

use crate::descending::{attestation, compile_chain};
use crate::policy::{
    compile_preprocessing_instructions, preprocess_to_stopping_point, PolicyLimits, PolicyOracle,
};
use crate::population::PopulationGraph;
use crate::representation::{
    BraidRepresentation, CheckpointedProofProgram, ProofInstruction, SemanticAction,
};
use crate::{population::ExpansionSeed, RepKey, Result};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct CohortRow {
    claim_id: String,
    source_knot: String,
    target_knot: String,
    strands: u16,
    source_word: Vec<i16>,
    cc_position: u32,
    target_word: Vec<i16>,
}

#[derive(Clone, Debug)]
pub struct CatalogueBraidImportRun {
    pub selected: usize,
    pub improved: usize,
    pub absent_sources: usize,
    pub already_sufficient: usize,
    pub failed: usize,
    pub collateral_improvements: usize,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub manifest: String,
}

pub fn import_catalogue_braid<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    cohort: &[u8],
    oracle: &mut O,
    limits: PolicyLimits,
    source_snapshot_sha256: &str,
) -> Result<CatalogueBraidImportRun> {
    let rows = parse_cohort(cohort)?;
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let before_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let mut improved = 0;
    let mut absent_sources = 0;
    let mut already_sufficient = 0;
    let mut failed = 0;
    let mut manifest = format!(
        "format\tunknotdb-catalogue-direct-braid-import-v0\n\
         source_snapshot_sha256\t{source_snapshot_sha256}\n\
         cohort_sha256\t{}\n\
         policy_model_id\t{}\n\
         policy_limits\t{}\t{}\n\
         result\tclaim_id\tsource_knot\ttarget_knot\tstatus\tsource_key\ttarget_key\told_u\tnew_u\tprogram_sha256\tdetail\n",
        unknotdb::util::sha256_hex(cohort),
        oracle.model_id(),
        limits.max_policy_plies,
        limits.max_semantic_moves,
    );

    for row in &rows {
        let result = compile_row(graph, row, oracle, limits);
        match result {
            Ok(CompiledDisposition::Improved {
                source_key,
                target_key,
                old_u,
                new_u,
                program_sha256,
                target_fallback_cc,
            }) => {
                improved += 1;
                push_result(
                    &mut manifest,
                    row,
                    "improved",
                    Some(source_key),
                    Some(target_key),
                    Some(old_u),
                    Some(new_u),
                    Some(program_sha256),
                    &format!(
                        "exact one-CC braid macro independently replayed; target_descending_cc={target_fallback_cc}"
                    ),
                );
            }
            Ok(CompiledDisposition::AbsentSource { source_key }) => {
                absent_sources += 1;
                push_result(
                    &mut manifest,
                    row,
                    "absent-source",
                    Some(source_key),
                    None,
                    None,
                    None,
                    None,
                    "preprocessed source stopping point is not in graph",
                );
            }
            Ok(CompiledDisposition::AlreadySufficient {
                source_key,
                target_key,
                current,
                proposed,
            }) => {
                already_sufficient += 1;
                push_result(
                    &mut manifest,
                    row,
                    "already-sufficient",
                    Some(source_key),
                    Some(target_key),
                    Some(current),
                    Some(current),
                    None,
                    &format!("candidate bound was {proposed}"),
                );
            }
            Err(error) => {
                failed += 1;
                push_result(
                    &mut manifest,
                    row,
                    "failed",
                    None,
                    None,
                    None,
                    None,
                    None,
                    &error.to_string(),
                );
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
    manifest.push_str(&format!(
        "summary\tselected\t{}\nsummary\timproved\t{improved}\n\
         summary\tabsent_sources\t{absent_sources}\n\
         summary\talready_sufficient\t{already_sufficient}\n\
         summary\tfailed\t{failed}\n\
         summary\tcollateral_improvements\t{collateral_improvements}\n\
         summary\tinserted_nodes\t{inserted_nodes}\n\
         summary\tinserted_edges\t{inserted_edges}\n",
        rows.len(),
    ));
    Ok(CatalogueBraidImportRun {
        selected: rows.len(),
        improved,
        absent_sources,
        already_sufficient,
        failed,
        collateral_improvements,
        inserted_nodes,
        inserted_edges,
        manifest,
    })
}

enum CompiledDisposition {
    Improved {
        source_key: RepKey,
        target_key: RepKey,
        old_u: u32,
        new_u: u32,
        program_sha256: RepKey,
        target_fallback_cc: u32,
    },
    AbsentSource {
        source_key: RepKey,
    },
    AlreadySufficient {
        source_key: RepKey,
        target_key: RepKey,
        current: u32,
        proposed: u32,
    },
}

fn compile_row<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    row: &CohortRow,
    oracle: &mut O,
    limits: PolicyLimits,
) -> Result<CompiledDisposition> {
    if row.source_word.len() != row.target_word.len()
        || row.cc_position as usize >= row.source_word.len()
    {
        return Err("invalid direct-braid cohort words/CC position".into());
    }
    let source_raw = BraidRepresentation {
        strands: row.strands,
        cyclic_band_generators: false,
        word: row.source_word.clone(),
    };
    let crossing = SemanticAction::CrossingChange {
        position: row.cc_position,
    };
    let target_raw = crossing.apply(&source_raw)?;
    if target_raw.word != row.target_word {
        return Err("recorded target word is not the exact one-CC successor".into());
    }
    let source_report = preprocess_to_stopping_point(&source_raw, oracle, limits)?;
    let target_report = preprocess_to_stopping_point(&target_raw, oracle, limits)?;
    if !source_report.is_graph_stopping_point() || !target_report.is_graph_stopping_point() {
        return Err("preprocessing did not reach policy stopping points".into());
    }
    let source_key = source_report.output.key;
    let target_key = target_report.output.key;
    let Some(old_u) = graph.u_upper_bound(&source_key) else {
        return Ok(CompiledDisposition::AbsentSource { source_key });
    };
    let mut staged = graph.clone();
    let target_fallback_cc = if staged.u_upper_bound(&target_key).is_none() {
        let seed = ExpansionSeed {
            representation: target_report.output.clone(),
            policy_stop: attestation(&target_report)?,
            u_upper_bound: u32::MAX,
            route_rank: u64::MAX,
            active_witness_semantic_len: 0,
        };
        let (edges, cc_count) = compile_chain(&seed, oracle, limits)?;
        for edge in edges.into_iter().rev() {
            let certificate_id = unknotdb::util::sha256(&edge.program.encode()?);
            staged.relax_unknot_edge_deferred_acs10(
                edge.source,
                edge.source_stop,
                edge.target_key,
                edge.program,
                Some(certificate_id),
            )?;
        }
        cc_count
    } else {
        0
    };
    let target_u = staged
        .u_upper_bound(&target_key)
        .ok_or("descending fallback did not connect target stopping point")?;
    let proposed = target_u.checked_add(1).ok_or("candidate U overflow")?;
    if proposed >= old_u {
        return Ok(CompiledDisposition::AlreadySufficient {
            source_key,
            target_key,
            current: old_u,
            proposed,
        });
    }
    let source_forward = CheckpointedProofProgram {
        instructions: compile_preprocessing_instructions(&source_report)?,
    };
    let mut instructions = if source_forward.instructions.is_empty() {
        Vec::new()
    } else {
        source_forward
            .invert_exact(&source_raw)?
            .program
            .instructions
    };
    instructions.push(ProofInstruction::Action(crossing));
    instructions.extend(compile_preprocessing_instructions(&target_report)?);
    let program = CheckpointedProofProgram { instructions }
        .canonicalize_semantic_first(&source_report.output.representation)?;
    if program.cc_cost() != 1
        || program.replay(&source_report.output.representation)?
            != target_report.output.representation
    {
        return Err("compiled direct-braid edge failed exact replay".into());
    }
    let program_sha256 = unknotdb::util::sha256(&program.encode()?);
    let source_stop = attestation(&source_report)?;
    staged.relax_unknot_edge_deferred_acs10(
        source_report.output,
        source_stop,
        target_key,
        program,
        None,
    )?;
    staged.recompute_unknot_routes()?;
    staged.recompute_acs10_routes()?;
    let new_u = staged
        .u_upper_bound(&source_key)
        .ok_or("source disappeared after relaxation")?;
    if new_u >= old_u || new_u != proposed {
        return Err("strict direct-braid relaxation produced an unexpected bound".into());
    }
    *graph = staged;
    Ok(CompiledDisposition::Improved {
        source_key,
        target_key,
        old_u,
        new_u,
        program_sha256,
        target_fallback_cc,
    })
}

fn parse_cohort(bytes: &[u8]) -> Result<Vec<CohortRow>> {
    let text = std::str::from_utf8(bytes)?;
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty direct-braid cohort")?;
    let columns: Vec<_> = header.split('\t').collect();
    let index = |name: &str| -> Result<usize> {
        columns
            .iter()
            .position(|column| *column == name)
            .ok_or_else(|| format!("cohort is missing {name}").into())
    };
    let claim_id = index("claim_id")?;
    let source_knot = index("source_knot")?;
    let target_knot = index("target_knot")?;
    let strands = index("strands")?;
    let source_word = index("source_word")?;
    let cc_position = index("cc_position")?;
    let target_word = index("target_word")?;
    lines
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(line_number, line)| -> Result<CohortRow> {
            let fields: Vec<_> = line.split('\t').collect();
            let field = |at: usize| -> Result<&str> {
                fields
                    .get(at)
                    .copied()
                    .ok_or_else(|| format!("short cohort row at line {}", line_number + 2).into())
            };
            Ok(CohortRow {
                claim_id: field(claim_id)?.to_owned(),
                source_knot: field(source_knot)?.to_owned(),
                target_knot: field(target_knot)?.to_owned(),
                strands: field(strands)?.parse()?,
                source_word: parse_word(field(source_word)?)?,
                cc_position: field(cc_position)?.parse()?,
                target_word: parse_word(field(target_word)?)?,
            })
        })
        .collect()
}

fn parse_word(value: &str) -> Result<Vec<i16>> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|item| item.parse().map_err(Into::into))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn push_result(
    manifest: &mut String,
    row: &CohortRow,
    status: &str,
    source_key: Option<RepKey>,
    target_key: Option<RepKey>,
    old_u: Option<u32>,
    new_u: Option<u32>,
    program_sha256: Option<RepKey>,
    detail: &str,
) {
    let optional_key = |value: Option<RepKey>| value.map_or_else(|| "-".to_owned(), |v| hex(&v));
    let optional_u = |value: Option<u32>| value.map_or_else(|| "-".to_owned(), |v| v.to_string());
    manifest.push_str(&format!(
        "result\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        row.claim_id,
        row.source_knot,
        row.target_knot,
        status,
        optional_key(source_key),
        optional_key(target_key),
        optional_u(old_u),
        optional_u(new_u),
        optional_key(program_sha256),
        detail.replace(['\t', '\n', '\r'], " "),
    ));
}

fn hex(bytes: &RepKey) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generated_cohort_contract() {
        let bytes = b"claim_id\tsource_knot\ttarget_knot\tstrands\tsource_word\tcc_position\ttarget_word\nabc\t12a_1\t0_1\t3\t1,-2,1\t1\t1,2,1\n";
        let rows = parse_cohort(bytes).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_word, vec![1, -2, 1]);
        assert_eq!(rows[0].target_word, vec![1, 2, 1]);
    }
}
