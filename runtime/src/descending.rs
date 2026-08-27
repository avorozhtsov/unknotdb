//! Exact descending-diagram fallback certificates for ordinary Artin braids.

use crate::policy::{
    compile_preprocessing_instructions, preprocess_to_stopping_point, preprocessing_audit_sha256,
    PolicyLimits, PolicyOracle, PolicyStopReason, PreprocessingReport,
};
use crate::population::{ExpansionSeed, PopulationGraph};
use crate::representation::{
    descending_crossing_changes_v0, BraidRepresentation, CheckpointedProofProgram,
    ProofInstruction, SemanticAction,
};
use crate::{PolicyStopAttestation, RepKey, Result};

pub const DESCENDING_CERTIFICATE_VERSION: &str = "unknotdb-descending-artin-v0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescendingBackfillLimits {
    pub min_u: u32,
    pub cohort_limit: usize,
    pub max_word_length: u32,
    pub full_recompute_threshold: usize,
    pub policy_limits: PolicyLimits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescendingBackfillRun {
    pub selected: usize,
    pub certified: usize,
    pub excluded: usize,
    pub failed: usize,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub direct_improvements: usize,
    pub collateral_improvements: usize,
    pub manifest: String,
}

#[derive(Clone)]
pub(crate) struct ChainEdge {
    pub(crate) source: crate::representation::NormalizedRepresentation,
    pub(crate) source_stop: PolicyStopAttestation,
    pub(crate) target_key: RepKey,
    pub(crate) program: CheckpointedProofProgram,
}

pub fn run_descending_backfill<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    oracle: &mut O,
    snapshot_id: &str,
    limits: DescendingBackfillLimits,
    selected_keys: Option<&[RepKey]>,
) -> Result<DescendingBackfillRun> {
    if limits.cohort_limit == 0 || limits.max_word_length == 0 {
        return Err("descending backfill limits must be positive".into());
    }
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let before_u: std::collections::BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let mut seeds = if let Some(keys) = selected_keys {
        let mut selected = Vec::new();
        for key in keys {
            if let Some(seed) = graph.expansion_seed(key)? {
                if seed.u_upper_bound >= limits.min_u {
                    selected.push(seed);
                }
            }
        }
        selected
    } else {
        graph
            .ranked_quality_seeds(graph.node_count())?
            .into_iter()
            .filter(|seed| seed.u_upper_bound >= limits.min_u)
            .take(limits.cohort_limit)
            .collect::<Vec<_>>()
    };
    seeds.sort_by_key(|seed| {
        (
            std::cmp::Reverse(seed.u_upper_bound),
            seed.representation.representation.strands,
            seed.representation.representation.word.len(),
            seed.representation.key,
        )
    });
    seeds.truncate(limits.cohort_limit);
    let mut manifest = format!(
        "{DESCENDING_CERTIFICATE_VERSION}\nsnapshot_id\t{snapshot_id}\nmodel_id\t{}\nconvention\ttop-strand-0\tdownward\tpositive-left-over\tclosure-bottom-i-to-top-i\nlimits\t{}\t{}\t{}\t{}\t{}\t{}\n",
        oracle.model_id(),
        limits.min_u,
        limits.cohort_limit,
        limits.max_word_length,
        limits.full_recompute_threshold,
        limits.policy_limits.max_policy_plies,
        limits.policy_limits.max_semantic_moves,
    );
    manifest.push_str(
        "certificate\tsource_key\told_u\tstrands\tword_length\tcc_count\tnew_u\tstatus\n",
    );
    let mut certified = 0;
    let mut excluded = 0;
    let mut failed = 0;
    let mut direct_improvements = 0;
    let mut staged = graph.clone();
    let mut successful = Vec::new();
    for seed in &seeds {
        let representation = &seed.representation.representation;
        if representation.cyclic_band_generators
            || representation.word.len() > limits.max_word_length as usize
        {
            excluded += 1;
            manifest.push_str(&format!(
                "certificate\t{}\t{}\t{}\t{}\t-\t{}\texcluded\n",
                hex(&seed.representation.key),
                seed.u_upper_bound,
                representation.strands,
                representation.word.len(),
                seed.u_upper_bound,
            ));
            continue;
        }
        match compile_chain(seed, oracle, limits.policy_limits) {
            Ok((edges, cc_count)) => {
                for edge in edges.into_iter().rev() {
                    let certificate_id = unknotdb::util::sha256(&edge.program.encode()?);
                    let outcome = staged.relax_unknot_edge_deferred_acs10(
                        edge.source,
                        edge.source_stop,
                        edge.target_key,
                        edge.program,
                        Some(certificate_id),
                    )?;
                    manifest.push_str(&format!("edge\t{:?}\n", outcome));
                }
                certified += 1;
                successful.push((
                    seed.representation.key,
                    seed.u_upper_bound,
                    representation.strands,
                    representation.word.len(),
                    cc_count,
                ));
            }
            Err(error) => {
                failed += 1;
                manifest.push_str(&format!(
                    "certificate\t{}\t{}\t{}\t{}\t-\t{}\tfailed:{}\n",
                    hex(&seed.representation.key),
                    seed.u_upper_bound,
                    representation.strands,
                    representation.word.len(),
                    seed.u_upper_bound,
                    sanitize(&error.to_string()),
                ));
            }
        }
    }
    staged.recompute_unknot_routes()?;
    staged.recompute_acs10_routes()?;
    for (key, old_u, strands, word_length, cc_count) in successful {
        let new_u = staged
            .u_upper_bound(&key)
            .ok_or("descending source disappeared after relaxation")?;
        if new_u > cc_count {
            return Err("descending certificate did not materialize its U bound".into());
        }
        direct_improvements += usize::from(new_u < old_u);
        manifest.push_str(&format!(
            "certificate\t{}\t{}\t{}\t{}\t{}\t{}\tverified\n",
            hex(&key),
            old_u,
            strands,
            word_length,
            cc_count,
            new_u,
        ));
    }
    *graph = staged;
    let after_u: std::collections::BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let collateral_improvements = after_u
        .iter()
        .filter(|(key, value)| before_u.get(*key).is_some_and(|old| **value < *old))
        .count()
        .saturating_sub(direct_improvements);
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        seeds.len(),
        certified,
        excluded,
        failed,
        inserted_nodes,
        inserted_edges,
        direct_improvements,
        collateral_improvements,
    ));
    Ok(DescendingBackfillRun {
        selected: seeds.len(),
        certified,
        excluded,
        failed,
        inserted_nodes,
        inserted_edges,
        direct_improvements,
        collateral_improvements,
        manifest,
    })
}

pub(crate) fn compile_chain<O: PolicyOracle>(
    seed: &ExpansionSeed,
    oracle: &mut O,
    policy_limits: PolicyLimits,
) -> Result<(Vec<ChainEdge>, u32)> {
    let changes = descending_crossing_changes_v0(&seed.representation.representation)?;
    let mut raw = seed.representation.representation.clone();
    let mut current = seed.representation.clone();
    let mut current_stop = seed.policy_stop;
    let mut previous_preprocessing: Option<(BraidRepresentation, CheckpointedProofProgram)> = None;
    let mut edges = Vec::new();
    for &position in &changes {
        let mut instructions = inverse_preprocessing(&previous_preprocessing)?;
        let action = SemanticAction::CrossingChange { position };
        instructions.push(ProofInstruction::Action(action));
        raw = action.apply(&raw)?;
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
        if program.replay(&current.representation)? != report.output.representation {
            return Err("descending CC edge failed exact replay".into());
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
    if !descending_crossing_changes_v0(&raw)?.is_empty() {
        return Err("selected crossing set did not produce a descending diagram".into());
    }
    let canonical_unknot = BraidRepresentation {
        strands: 1,
        cyclic_band_generators: false,
        word: Vec::new(),
    }
    .normalize()?;
    if current.key != canonical_unknot.key {
        let mut instructions = inverse_preprocessing(&previous_preprocessing)?;
        instructions.push(ProofInstruction::Action(SemanticAction::DescendingCollapse));
        let program = CheckpointedProofProgram { instructions }
            .canonicalize_semantic_first(&current.representation)?;
        if program.replay(&current.representation)? != canonical_unknot.representation {
            return Err("descending terminal macro failed exact replay".into());
        }
        edges.push(ChainEdge {
            source: current,
            source_stop: current_stop,
            target_key: canonical_unknot.key,
            program,
        });
    }
    Ok((edges, changes.len().try_into()?))
}

fn inverse_preprocessing(
    previous: &Option<(BraidRepresentation, CheckpointedProofProgram)>,
) -> Result<Vec<ProofInstruction>> {
    let Some((raw, forward)) = previous else {
        return Ok(Vec::new());
    };
    if forward.instructions.is_empty() {
        return Ok(Vec::new());
    }
    // A preprocessing witness may be coordinate-only. Keep its exact inverse
    // here; the enclosing edge has a following CC or DESCENDING_COLLAPSE and
    // canonicalizes the complete composite to a semantic-first program.
    Ok(forward.invert_exact(raw)?.program.instructions)
}

pub(crate) fn attestation(report: &PreprocessingReport) -> Result<PolicyStopAttestation> {
    let audit_sha256 = preprocessing_audit_sha256(report)?;
    match report.stop_reason {
        PolicyStopReason::PreferredCrossingChange => {
            Ok(PolicyStopAttestation::PreferredCrossingChange {
                action: report
                    .preferred_crossing_change
                    .ok_or("CC stop lacks action")?
                    .encode_u63()?,
                audit_sha256,
            })
        }
        PolicyStopReason::TerminalRepresentation => {
            Ok(PolicyStopAttestation::Terminal { audit_sha256 })
        }
        _ => Err("incomplete preprocessing has no graph attestation".into()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sanitize(value: &str) -> String {
    value.replace(['\t', '\n'], " ")
}
