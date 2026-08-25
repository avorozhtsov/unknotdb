//! Exhaustive small natural-braid census for selecting bounded population limits.

use crate::frontier::{enumerate_scrambles_detailed, ScrambleLimits};
#[cfg(test)]
use crate::policy::preprocess_to_stopping_point;
use crate::policy::{
    compile_preprocessing_instructions, preprocess_many_to_stopping_points,
    preprocessing_audit_sha256, trace_crossing_change_edge_from_attestation, PolicyLimits,
    PolicyOracle, PolicyStopReason,
};
use crate::population::{PopulationGraph, RelaxationOutcome};
use crate::reducer::DeterministicReducer;
use crate::representation::{
    BraidRepresentation, CheckpointedProofProgram, NormalizedRepresentation, SemanticAction,
};
use crate::{GraphSnapshot, RepKey, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const NATURAL_CENSUS_VERSION: &str = "unknotdb-natural-braid-census-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaturalBraidSeed {
    pub representation: NormalizedRepresentation,
    pub raw_multiplicity: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaturalBraidEnumeration {
    pub max_strands: u16,
    pub max_word_length: u32,
    pub raw_knot_inputs: u64,
    pub seeds: Vec<NaturalBraidSeed>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Distribution {
    pub p50: u32,
    pub p95: u32,
    pub p99: u32,
    pub max: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusSeedResult {
    pub seed: NaturalBraidSeed,
    pub reducer_actions: u32,
    pub l10_before: u64,
    pub l10_after: u64,
    pub policy_plies: u32,
    pub semantic_moves: u32,
    pub policy_rounds: u32,
    pub stop_reason: PolicyStopReason,
    pub preferred_crossing_change: Option<SemanticAction>,
    pub stopping: NormalizedRepresentation,
    pub graph_stopping_point: bool,
    pub graph_hit: bool,
    pub u_upper_bound: Option<u32>,
    pub acs10: Option<u32>,
    pub preprocessing_micros: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontierMeasurement {
    pub stopping_key: RepKey,
    pub strands: u16,
    pub word_length: u32,
    pub visited_states: u32,
    pub candidates: u32,
    pub truncated_by_states: bool,
    pub truncated_by_candidates: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaturalBraidCensus {
    pub model_id: String,
    pub snapshot_source_generation: String,
    pub snapshot_nodes: usize,
    pub snapshot_edges: usize,
    pub enumeration: NaturalBraidEnumeration,
    pub measurement_policy_limits: PolicyLimits,
    pub recommended_policy_limits: PolicyLimits,
    pub policy_replay_matches: bool,
    pub distinct_reducer_outputs: usize,
    pub measurement_preprocessing_runs: usize,
    pub replay_preprocessing_runs: usize,
    pub scramble_depth: u16,
    pub measurement_scramble_limits: ScrambleLimits,
    pub recommended_scramble_limits: ScrambleLimits,
    pub seeds: Vec<CensusSeedResult>,
    pub frontier: Vec<FrontierMeasurement>,
}

struct DeduplicatedPreprocessing {
    reports: Vec<(crate::policy::PreprocessingReport, u64)>,
    distinct_reducer_outputs: usize,
}

fn preprocess_seeds_deduplicated<O: PolicyOracle>(
    seeds: &[NaturalBraidSeed],
    oracle: &mut O,
    limits: PolicyLimits,
) -> Result<DeduplicatedPreprocessing> {
    let started = Instant::now();
    let mut reductions = Vec::with_capacity(seeds.len());
    let mut distinct = BTreeMap::<RepKey, BraidRepresentation>::new();
    for seed in seeds {
        let reduction = DeterministicReducer::reduce(&seed.representation.representation)?;
        distinct
            .entry(reduction.output.key)
            .or_insert_with(|| reduction.output.representation.clone());
        reductions.push(reduction);
    }
    let distinct_inputs: Vec<_> = distinct.values().cloned().collect();
    let templates = preprocess_many_to_stopping_points(&distinct_inputs, oracle, limits)?;
    let mut suffixes = BTreeMap::<RepKey, crate::policy::PreprocessingReport>::new();
    for (key, report) in distinct.keys().copied().zip(templates) {
        if report.initial_reduction.output.key != key
            || !report.initial_reduction.program.actions.is_empty()
        {
            return Err("reducer output was not an idempotent preprocessing source".into());
        }
        suffixes.insert(key, report);
    }
    let total_micros = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    let average_micros = total_micros / seeds.len().max(1) as u64;
    let mut reports = Vec::with_capacity(seeds.len());
    for (seed, reduction) in seeds.iter().zip(reductions) {
        let mut report = suffixes
            .get(&reduction.output.key)
            .ok_or("missing batched preprocessing template")?
            .clone();
        report.input = seed.representation.representation.clone();
        report.initial_reduction = reduction;
        let first_round = report
            .rounds
            .first_mut()
            .ok_or("preprocessing report has no policy round")?;
        first_round.source = report.initial_reduction.output.clone();
        if report.rounds.len() == 1 {
            report.output = report.initial_reduction.output.clone();
        }
        report.verify()?;
        reports.push((report, average_micros));
    }
    Ok(DeduplicatedPreprocessing {
        reports,
        distinct_reducer_outputs: suffixes.len(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaturalBraidCompletionEdge {
    pub source_key: RepKey,
    pub target_key: RepKey,
    pub u_upper_bound: u32,
    pub cc_cost: u32,
    pub source_audit_sha256: RepKey,
    pub program: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaturalBraidCompletion {
    pub model_id: String,
    pub max_strands: u16,
    pub max_word_length: u32,
    pub normalized_seeds: usize,
    pub distinct_stopping_points: usize,
    pub missing_before: usize,
    pub graph_hits_after: usize,
    pub inserted_edges: Vec<NaturalBraidCompletionEdge>,
}

#[derive(Clone, Debug)]
struct CompletionEdgeTrace {
    source: NormalizedRepresentation,
    source_attestation: crate::policy::PreprocessingReport,
    target: NormalizedRepresentation,
    successor_preprocessing: crate::policy::PreprocessingReport,
    program: CheckpointedProofProgram,
}

pub fn enumerate_natural_braids(
    max_strands: u16,
    max_word_length: u32,
) -> Result<NaturalBraidEnumeration> {
    if !(1..=4).contains(&max_strands) {
        return Err("natural braid census v1 supports 1 <= max_strands <= 4".into());
    }
    if max_word_length > 10 {
        return Err("natural braid census v1 caps word length at 10".into());
    }
    let mut by_key = BTreeMap::<RepKey, NaturalBraidSeed>::new();
    let unknot = BraidRepresentation {
        strands: 1,
        cyclic_band_generators: false,
        word: vec![],
    }
    .normalize()?;
    by_key.insert(
        unknot.key,
        NaturalBraidSeed {
            representation: unknot,
            raw_multiplicity: 1,
        },
    );
    let mut raw_knot_inputs = 1_u64;
    for strands in 2..=max_strands {
        let alphabet_size = 2_u64 * u64::from(strands - 1);
        for length in 1..=max_word_length {
            let word_count = alphabet_size
                .checked_pow(length)
                .ok_or("natural braid word enumeration overflow")?;
            for mut ordinal in 0..word_count {
                let mut word = Vec::with_capacity(length as usize);
                for _ in 0..length {
                    let digit = ordinal % alphabet_size;
                    ordinal /= alphabet_size;
                    let generator: i16 = (digit / 2 + 1)
                        .try_into()
                        .map_err(|_| "natural braid generator does not fit i16")?;
                    word.push(if digit.is_multiple_of(2) {
                        generator
                    } else {
                        -generator
                    });
                }
                let representation = BraidRepresentation {
                    strands,
                    cyclic_band_generators: false,
                    word,
                };
                if !representation.is_knot_closure()? {
                    continue;
                }
                raw_knot_inputs = raw_knot_inputs
                    .checked_add(1)
                    .ok_or("raw natural input count overflow")?;
                let normalized = representation.normalize()?;
                by_key
                    .entry(normalized.key)
                    .and_modify(|seed| seed.raw_multiplicity += 1)
                    .or_insert(NaturalBraidSeed {
                        representation: normalized,
                        raw_multiplicity: 1,
                    });
            }
        }
    }
    Ok(NaturalBraidEnumeration {
        max_strands,
        max_word_length,
        raw_knot_inputs,
        seeds: by_key.into_values().collect(),
    })
}

pub fn run_natural_braid_census<O: PolicyOracle>(
    oracle: &mut O,
    snapshot: &GraphSnapshot,
    max_strands: u16,
    max_word_length: u32,
    measurement_policy_limits: PolicyLimits,
    scramble_depth: u16,
) -> Result<NaturalBraidCensus> {
    if snapshot.meta().synthetic {
        return Err("natural-braid census needs a production snapshot".into());
    }
    if snapshot.meta().policy_model_id.as_deref() != Some(oracle.model_id()) {
        return Err("census snapshot and oracle policy model IDs differ".into());
    }
    if scramble_depth == 0 {
        return Err("census scramble depth must be positive".into());
    }
    let enumeration = enumerate_natural_braids(max_strands, max_word_length)?;
    let measured =
        preprocess_seeds_deduplicated(&enumeration.seeds, oracle, measurement_policy_limits)?;
    let measurement_preprocessing_runs = measured.distinct_reducer_outputs;
    let mut seeds = Vec::with_capacity(enumeration.seeds.len());
    for (seed, (report, preprocessing_micros)) in enumeration.seeds.iter().zip(measured.reports) {
        let route = report
            .is_graph_stopping_point()
            .then(|| snapshot.lookup(&report.output.key))
            .flatten();
        let graph_stopping_point = report.is_graph_stopping_point();
        seeds.push(CensusSeedResult {
            seed: seed.clone(),
            reducer_actions: report
                .initial_reduction
                .program
                .actions
                .len()
                .try_into()
                .map_err(|_| "reducer action count does not fit u32")?,
            l10_before: report.initial_reduction.l10_before,
            l10_after: report.initial_reduction.l10_after,
            policy_plies: report.policy_plies,
            semantic_moves: report.semantic_moves,
            policy_rounds: report
                .rounds
                .len()
                .try_into()
                .map_err(|_| "policy round count does not fit u32")?,
            stop_reason: report.stop_reason,
            preferred_crossing_change: report.preferred_crossing_change,
            stopping: report.output,
            graph_stopping_point,
            graph_hit: route.is_some(),
            u_upper_bound: route.and_then(|route| route.u_upper_bound),
            acs10: route.and_then(|route| route.acs10),
            preprocessing_micros,
        });
    }
    if seeds.iter().any(|seed| {
        matches!(
            seed.stop_reason,
            PolicyStopReason::PolicyPlyLimit | PolicyStopReason::SemanticMoveLimit
        )
    }) {
        return Err("measurement policy limits truncated the natural-braid census".into());
    }

    let max_policy_plies = seeds
        .iter()
        .map(|seed| seed.policy_plies)
        .max()
        .unwrap_or(0);
    let max_semantic_moves = seeds
        .iter()
        .map(|seed| seed.semantic_moves)
        .max()
        .unwrap_or(0);
    let recommended_policy_limits = PolicyLimits {
        max_policy_plies: with_power_of_two_headroom(max_policy_plies, 16)?,
        max_semantic_moves: with_power_of_two_headroom(max_semantic_moves, 4)?,
    };

    let replayed =
        preprocess_seeds_deduplicated(&enumeration.seeds, oracle, recommended_policy_limits)?;
    let replay_preprocessing_runs = replayed.distinct_reducer_outputs;
    for (measured, (replay, _)) in seeds.iter().zip(replayed.reports) {
        if replay.stop_reason != measured.stop_reason
            || replay.output != measured.stopping
            || replay.policy_plies != measured.policy_plies
            || replay.semantic_moves != measured.semantic_moves
            || replay.preferred_crossing_change != measured.preferred_crossing_change
        {
            return Err(
                "recommended policy limits changed a natural-braid preprocessing result".into(),
            );
        }
    }

    let mut stopping_points = BTreeMap::new();
    for seed in &seeds {
        if seed.graph_stopping_point {
            stopping_points
                .entry(seed.stopping.key)
                .or_insert_with(|| seed.stopping.clone());
        }
    }
    let max_stopping_strands = stopping_points
        .values()
        .map(|point| point.representation.strands)
        .max()
        .unwrap_or(2);
    let max_stopping_word = stopping_points
        .values()
        .map(|point| point.representation.word.len() as u32)
        .max()
        .unwrap_or(max_word_length);
    let measurement_scramble_limits = ScrambleLimits {
        depth: scramble_depth,
        max_states: 1_000_000,
        max_candidates: 1_000_000,
        max_strands: max_stopping_strands
            .checked_add(scramble_depth)
            .ok_or("census scramble strand bound overflow")?,
        max_word_length: max_stopping_word
            .checked_add(2 * u32::from(scramble_depth))
            .ok_or("census scramble word bound overflow")?,
        min_cc_cost: 1,
        max_cc_cost: 1,
    };
    let mut frontier = Vec::with_capacity(stopping_points.len());
    for stopping in stopping_points.values() {
        let enumeration =
            enumerate_scrambles_detailed(&stopping.representation, measurement_scramble_limits)?;
        frontier.push(FrontierMeasurement {
            stopping_key: stopping.key,
            strands: stopping.representation.strands,
            word_length: stopping
                .representation
                .word
                .len()
                .try_into()
                .map_err(|_| "stopping word length does not fit u32")?,
            visited_states: enumeration.visited_states,
            candidates: enumeration
                .candidates
                .len()
                .try_into()
                .map_err(|_| "scramble candidate count does not fit u32")?,
            truncated_by_states: enumeration.truncated_by_states,
            truncated_by_candidates: enumeration.truncated_by_candidates,
        });
    }
    if frontier
        .iter()
        .any(|row| row.truncated_by_states || row.truncated_by_candidates)
    {
        return Err("measurement scramble limits truncated the natural-braid census".into());
    }
    let max_states = frontier
        .iter()
        .map(|row| row.visited_states)
        .max()
        .unwrap_or(1);
    let max_candidates = frontier.iter().map(|row| row.candidates).max().unwrap_or(1);
    let recommended_scramble_limits = ScrambleLimits {
        max_states: with_power_of_two_headroom(max_states, 64)?,
        max_candidates: with_power_of_two_headroom(max_candidates, 16)?,
        ..measurement_scramble_limits
    };

    Ok(NaturalBraidCensus {
        model_id: oracle.model_id().to_owned(),
        snapshot_source_generation: snapshot.meta().source_generation.clone(),
        snapshot_nodes: snapshot.node_count(),
        snapshot_edges: snapshot.edge_count(),
        enumeration,
        measurement_policy_limits,
        recommended_policy_limits,
        policy_replay_matches: true,
        distinct_reducer_outputs: measurement_preprocessing_runs,
        measurement_preprocessing_runs,
        replay_preprocessing_runs,
        scramble_depth,
        measurement_scramble_limits,
        recommended_scramble_limits,
        seeds,
        frontier,
    })
}

pub fn complete_natural_braids<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    oracle: &mut O,
    max_strands: u16,
    max_word_length: u32,
    policy_limits: PolicyLimits,
) -> Result<NaturalBraidCompletion> {
    let enumeration = enumerate_natural_braids(max_strands, max_word_length)?;
    let preprocessed = preprocess_seeds_deduplicated(&enumeration.seeds, oracle, policy_limits)?;
    let mut stopping_points = BTreeMap::<RepKey, crate::policy::PreprocessingReport>::new();
    let mut seed_stops = Vec::with_capacity(enumeration.seeds.len());
    for (seed, (report, _)) in enumeration.seeds.iter().zip(preprocessed.reports) {
        if !report.is_graph_stopping_point() {
            return Err(format!(
                "natural braid seed {} did not reach a graph stopping point: {:?}",
                hex(&seed.representation.key),
                report.stop_reason
            )
            .into());
        }
        seed_stops.push(report.output.key);
        stopping_points.entry(report.output.key).or_insert(report);
    }
    let missing_before = stopping_points
        .keys()
        .filter(|key| graph.u_upper_bound(key).is_none())
        .count();
    let mut pending: Vec<_> = stopping_points
        .into_values()
        .filter(|report| graph.u_upper_bound(&report.output.key).is_none())
        .collect();
    pending.sort_by_key(|report| {
        (
            report.output.representation.strands,
            report.output.representation.word.len(),
            report.output.key,
        )
    });
    let mut inserted_edges = Vec::new();
    for source_attestation in pending {
        if graph
            .u_upper_bound(&source_attestation.output.key)
            .is_some()
        {
            continue;
        }
        let source_key = source_attestation.output.key;
        let mut seen = BTreeSet::from([source_key]);
        let mut dead = BTreeSet::new();
        let path = find_cc_path_to_graph(
            graph,
            source_attestation,
            oracle,
            &mut seen,
            &mut dead,
            1_000,
        )?
        .ok_or_else(|| {
            format!(
                "natural completion found no bounded acyclic CC route from {}",
                hex(&source_key)
            )
        })?;

        for trace in path.into_iter().rev() {
            let source_audit_sha256 = preprocessing_audit_sha256(&trace.source_attestation)?;
            let cc_cost = trace.program.cc_cost();
            let program = trace.program.encode()?;
            let outcome = graph.relax_unknot_edge_deferred_acs10(
                trace.source.clone(),
                crate::PolicyStopAttestation::PreferredCrossingChange {
                    action: trace
                        .source_attestation
                        .preferred_crossing_change
                        .ok_or("completion source attestation has no preferred CC")?
                        .encode_u63()?,
                    audit_sha256: source_audit_sha256,
                },
                trace.target.key,
                trace.program,
                None,
            )?;
            let u_upper_bound = match outcome {
                RelaxationOutcome::Inserted { u_upper_bound }
                | RelaxationOutcome::Improved {
                    new: u_upper_bound, ..
                } => u_upper_bound,
                other => {
                    return Err(format!(
                        "missing natural stopping point was not inserted: {other:?}"
                    )
                    .into())
                }
            };
            inserted_edges.push(NaturalBraidCompletionEdge {
                source_key: trace.source.key,
                target_key: trace.target.key,
                u_upper_bound,
                cc_cost,
                source_audit_sha256,
                program,
            });
        }
    }
    graph.recompute_acs10_routes()?;
    let graph_hits_after = seed_stops
        .iter()
        .filter(|key| graph.u_upper_bound(key).is_some())
        .count();
    if graph_hits_after != enumeration.seeds.len() {
        return Err("natural completion did not cover every normalized seed".into());
    }
    Ok(NaturalBraidCompletion {
        model_id: oracle.model_id().to_owned(),
        max_strands,
        max_word_length,
        normalized_seeds: enumeration.seeds.len(),
        distinct_stopping_points: seed_stops.into_iter().collect::<BTreeSet<_>>().len(),
        missing_before,
        graph_hits_after,
        inserted_edges,
    })
}

fn find_cc_path_to_graph<O: PolicyOracle>(
    graph: &PopulationGraph,
    attestation: crate::policy::PreprocessingReport,
    oracle: &mut O,
    seen: &mut BTreeSet<RepKey>,
    dead: &mut BTreeSet<RepKey>,
    remaining_crossing_changes: u32,
) -> Result<Option<Vec<CompletionEdgeTrace>>> {
    if remaining_crossing_changes == 0 {
        return Ok(None);
    }
    let source_key = attestation.output.key;
    let preferred = attestation
        .preferred_crossing_change
        .ok_or("non-terminal natural completion source has no preferred CC")?;
    let mut actions = vec![preferred];
    actions.extend(
        (0..attestation.output.representation.word.len() as u32)
            .map(|position| SemanticAction::CrossingChange { position })
            .filter(|action| *action != preferred),
    );
    for action in actions {
        let policy_trace =
            trace_crossing_change_edge_from_attestation(attestation.clone(), action, oracle)?;
        let trace = CompletionEdgeTrace {
            source: policy_trace.source,
            source_attestation: policy_trace.source_attestation,
            target: policy_trace.target,
            successor_preprocessing: policy_trace.successor_preprocessing,
            program: policy_trace.program,
        };
        if graph.u_upper_bound(&trace.target.key).is_some() {
            return Ok(Some(vec![trace]));
        }
        if seen.contains(&trace.target.key) || dead.contains(&trace.target.key) {
            continue;
        }
        seen.insert(trace.target.key);
        let successor = trace.successor_preprocessing.clone();
        if let Some(mut suffix) = find_cc_path_to_graph(
            graph,
            successor,
            oracle,
            seen,
            dead,
            remaining_crossing_changes - 1,
        )? {
            let mut path = Vec::with_capacity(suffix.len() + 1);
            path.push(trace);
            path.append(&mut suffix);
            return Ok(Some(path));
        }
        seen.remove(&trace.target.key);
    }

    // A stopping point only says that the policy currently prefers a CC. It
    // does not imply that applying a CC in the current diagram connects every
    // component of the stopping graph. Bounded proof macros may expose a
    // different crossing by zero-cost moves before their single CC.
    let source = attestation.output.clone();
    let source_word_length: u32 = source
        .representation
        .word
        .len()
        .try_into()
        .map_err(|_| "completion source word length does not fit u32")?;
    for macro_depth in 2..=4 {
        let macro_limits = ScrambleLimits {
            depth: macro_depth,
            max_states: 32_768,
            max_candidates: 4_096,
            max_strands: source.representation.strands.saturating_add(1),
            max_word_length: source_word_length.saturating_add(4),
            min_cc_cost: 1,
            max_cc_cost: 1,
        };
        let macros = enumerate_scrambles_detailed(&source.representation, macro_limits)?;
        let inputs = macros
            .candidates
            .iter()
            .map(|candidate| candidate.output.clone())
            .collect::<Vec<_>>();
        let reports = preprocess_many_to_stopping_points(&inputs, oracle, attestation.limits)?;
        for (candidate, successor_preprocessing) in macros.candidates.into_iter().zip(reports) {
            if !successor_preprocessing.is_graph_stopping_point() {
                continue;
            }
            let target = successor_preprocessing.output.clone();
            if seen.contains(&target.key) || dead.contains(&target.key) {
                continue;
            }
            let mut instructions = candidate.program().instructions;
            instructions.extend(compile_preprocessing_instructions(
                &successor_preprocessing,
            )?);
            let program = CheckpointedProofProgram { instructions };
            if program.cc_cost() != 1
                || program.replay(&source.representation)? != target.representation
            {
                return Err("bounded completion macro failed exact replay".into());
            }
            let trace = CompletionEdgeTrace {
                source: source.clone(),
                source_attestation: attestation.clone(),
                target: target.clone(),
                successor_preprocessing: successor_preprocessing.clone(),
                program,
            };
            if graph.u_upper_bound(&target.key).is_some() {
                return Ok(Some(vec![trace]));
            }
            seen.insert(target.key);
            if let Some(mut suffix) = find_cc_path_to_graph(
                graph,
                successor_preprocessing,
                oracle,
                seen,
                dead,
                remaining_crossing_changes - 1,
            )? {
                let mut path = Vec::with_capacity(suffix.len() + 1);
                path.push(trace);
                path.append(&mut suffix);
                return Ok(Some(path));
            }
            seen.remove(&target.key);
        }
    }
    dead.insert(source_key);
    Ok(None)
}

impl NaturalBraidCensus {
    pub fn policy_plies_distribution(&self) -> Distribution {
        distribution(self.seeds.iter().map(|seed| seed.policy_plies))
    }

    pub fn semantic_moves_distribution(&self) -> Distribution {
        distribution(self.seeds.iter().map(|seed| seed.semantic_moves))
    }

    pub fn manifest_tsv(&self) -> String {
        let mut text = String::new();
        text.push_str(NATURAL_CENSUS_VERSION);
        text.push('\n');
        text.push_str(&format!("model_id\t{}\n", self.model_id));
        text.push_str(&format!(
            "snapshot\t{}\t{}\t{}\n",
            self.snapshot_source_generation, self.snapshot_nodes, self.snapshot_edges
        ));
        text.push_str(&format!(
            "input_bounds\t{}\t{}\t{}\t{}\n",
            self.enumeration.max_strands,
            self.enumeration.max_word_length,
            self.enumeration.raw_knot_inputs,
            self.enumeration.seeds.len()
        ));
        text.push_str(&format!(
            "policy_limits\tmeasurement\t{}\t{}\npolicy_limits\trecommended\t{}\t{}\n",
            self.measurement_policy_limits.max_policy_plies,
            self.measurement_policy_limits.max_semantic_moves,
            self.recommended_policy_limits.max_policy_plies,
            self.recommended_policy_limits.max_semantic_moves
        ));
        text.push_str(&format!(
            "preprocessing_dedup\t{}\t{}\t{}\n",
            self.distinct_reducer_outputs,
            self.measurement_preprocessing_runs,
            self.replay_preprocessing_runs
        ));
        text.push_str(&format!(
            "scramble_limits\tmeasurement\t{}\t{}\t{}\t{}\t{}\n",
            self.measurement_scramble_limits.depth,
            self.measurement_scramble_limits.max_states,
            self.measurement_scramble_limits.max_candidates,
            self.measurement_scramble_limits.max_strands,
            self.measurement_scramble_limits.max_word_length
        ));
        text.push_str(&format!(
            "scramble_limits\trecommended\t{}\t{}\t{}\t{}\t{}\n",
            self.recommended_scramble_limits.depth,
            self.recommended_scramble_limits.max_states,
            self.recommended_scramble_limits.max_candidates,
            self.recommended_scramble_limits.max_strands,
            self.recommended_scramble_limits.max_word_length
        ));
        text.push_str("seed\trep_key\tstrands\tword_length\tword\traw_multiplicity\treducer_actions\tl10_before\tl10_after\tpolicy_plies\tsemantic_moves\tpolicy_rounds\tstop_reason\tstopping_key\tstopping_strands\tstopping_word_length\tgraph_stopping\tgraph_hit\tu_upper\tacs10\tpreprocess_us\n");
        for row in &self.seeds {
            text.push_str(&format!(
                "seed\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                hex(&row.seed.representation.key),
                row.seed.representation.representation.strands,
                row.seed.representation.representation.word.len(),
                word(&row.seed.representation.representation.word),
                row.seed.raw_multiplicity,
                row.reducer_actions,
                row.l10_before,
                row.l10_after,
                row.policy_plies,
                row.semantic_moves,
                row.policy_rounds,
                row.stop_reason,
                hex(&row.stopping.key),
                row.stopping.representation.strands,
                row.stopping.representation.word.len(),
                u8::from(row.graph_stopping_point),
                u8::from(row.graph_hit),
                optional_u32(row.u_upper_bound),
                optional_u32(row.acs10),
                row.preprocessing_micros
            ));
        }
        text.push_str("frontier\tstopping_key\tstrands\tword_length\tvisited_states\tcandidates\ttruncated_states\ttruncated_candidates\n");
        for row in &self.frontier {
            text.push_str(&format!(
                "frontier\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                hex(&row.stopping_key),
                row.strands,
                row.word_length,
                row.visited_states,
                row.candidates,
                u8::from(row.truncated_by_states),
                u8::from(row.truncated_by_candidates)
            ));
        }
        text
    }

    pub fn report_markdown(&self) -> String {
        let plies = self.policy_plies_distribution();
        let moves = self.semantic_moves_distribution();
        let preprocessing = distribution(
            self.seeds
                .iter()
                .map(|seed| seed.preprocessing_micros.min(u64::from(u32::MAX)) as u32),
        );
        let admitted = self
            .seeds
            .iter()
            .filter(|seed| seed.graph_stopping_point)
            .count();
        let hits = self.seeds.iter().filter(|seed| seed.graph_hit).count();
        let unique_stops = self
            .seeds
            .iter()
            .filter(|seed| seed.graph_stopping_point)
            .map(|seed| seed.stopping.key)
            .collect::<BTreeSet<_>>()
            .len();
        let max_reducer = self
            .seeds
            .iter()
            .map(|seed| seed.reducer_actions)
            .max()
            .unwrap_or(0);
        let max_states = self
            .frontier
            .iter()
            .map(|row| row.visited_states)
            .max()
            .unwrap_or(0);
        let max_candidates = self
            .frontier
            .iter()
            .map(|row| row.candidates)
            .max()
            .unwrap_or(0);
        let mut reasons = BTreeMap::<String, usize>::new();
        for seed in &self.seeds {
            *reasons
                .entry(format!("{:?}", seed.stop_reason))
                .or_default() += 1;
        }
        let reason_rows = reasons
            .into_iter()
            .map(|(reason, count)| format!("| `{reason}` | {count} |"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut missing = BTreeMap::<RepKey, (&NormalizedRepresentation, usize, u64)>::new();
        for seed in self.seeds.iter().filter(|seed| !seed.graph_hit) {
            missing
                .entry(seed.stopping.key)
                .and_modify(|row| {
                    row.1 += 1;
                    row.2 += seed.seed.raw_multiplicity;
                })
                .or_insert((&seed.stopping, 1, seed.seed.raw_multiplicity));
        }
        let missing_rows = if missing.is_empty() {
            "| none | - | - | 0 | 0 |".into()
        } else {
            missing
                .into_iter()
                .map(|(key, (stopping, seed_count, raw_count))| {
                    format!(
                        "| `{}` | B{} | `{}` | {} | {} |",
                        hex(&key),
                        stopping.representation.strands,
                        word(&stopping.representation.word),
                        seed_count,
                        raw_count
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "# Natural braid census and pilot limits\n\n\
             Contract: `{}` under `{}`. Inputs use the ordinary Artin alphabet, cyclic-word origin normalization and mirror-orbit keys. No model was trained.\n\n\
             ## Coverage\n\n\
             - Input bounds: `strands <= {}`, `word_length <= {}`.\n\
             - Raw knot-closing inputs including `B1 []`: {}.\n\
             - Distinct normalized seeds: {}.\n\
             - Admitted preprocessing results: {}; distinct stopping points: {}.\n\
             - Exact preprocessed-key hits in the {}-node parent snapshot: {} of {} normalized seeds.\n\n\
             | Stop reason | Seeds |\n|---|---:|\n{}\n\n\
             Missing stopping points:\n\n\
             | Stopping key | Braid | Word | Normalized seeds | Raw inputs |\n|---|---:|---|---:|---:|\n{}\n\n\
             ## Observed preprocessing\n\n\
             | Metric | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|\n\
             | Policy plies | {} | {} | {} | {} |\n\
             | Semantic zero-CC moves | {} | {} | {} | {} |\n\
             | Preprocessing time, us | {} | {} | {} | {} |\n\n\
             Timing is the grouped preprocessing wall time divided evenly across normalized inputs; it includes reducer work, batched Q254 inference and certificate assembly. The {} normalized inputs collapsed to {} distinct reducer outputs, so the measurement and replay passes each ran preprocessing policy {} times instead of {} times. Maximum deterministic reducer program: {} actions. The proposed limits reproduced every stopping key, reason and move count from the broad measurement run.\n\n\
             ## Pilot limits for strands <= {}, word length <= {}\n\n\
             - `max_policy_plies = {}`\n\
             - `max_semantic_moves = {}`\n\
             - scramble depth `{}`, `max_scramble_states = {}`, `max_scramble_candidates = {}`\n\
             - scramble geometry bounds: `max_strands = {}`, `max_word_length = {}`\n\n\
             Exhaustive depth-{} measurements over {} distinct stopping points observed at most {} states and {} candidates, without measurement-cap truncation. The geometry bounds allow every stopping point plus up to one stabilization per ply or a two-letter insertion per ply. These remain pilot limits for the stated input bounds.\n",
            self.model_id,
            self.snapshot_source_generation,
            self.enumeration.max_strands,
            self.enumeration.max_word_length,
            self.enumeration.raw_knot_inputs,
            self.enumeration.seeds.len(),
            admitted,
            unique_stops,
            self.snapshot_nodes,
            hits,
            self.seeds.len(),
            reason_rows,
            missing_rows,
            plies.p50,
            plies.p95,
            plies.p99,
            plies.max,
            moves.p50,
            moves.p95,
            moves.p99,
            moves.max,
            preprocessing.p50,
            preprocessing.p95,
            preprocessing.p99,
            preprocessing.max,
            self.enumeration.seeds.len(),
            self.distinct_reducer_outputs,
            self.measurement_preprocessing_runs,
            self.enumeration.seeds.len(),
            max_reducer,
            self.enumeration.max_strands,
            self.enumeration.max_word_length,
            self.recommended_policy_limits.max_policy_plies,
            self.recommended_policy_limits.max_semantic_moves,
            self.recommended_scramble_limits.depth,
            self.recommended_scramble_limits.max_states,
            self.recommended_scramble_limits.max_candidates,
            self.recommended_scramble_limits.max_strands,
            self.recommended_scramble_limits.max_word_length,
            self.scramble_depth,
            self.frontier.len(),
            max_states,
            max_candidates
        )
    }
}

impl NaturalBraidCompletion {
    pub fn manifest_tsv(&self, parent_source_generation: &str) -> String {
        let mut text = String::new();
        text.push_str("unknotdb-natural-braid-completion-v1\n");
        text.push_str(&format!("model_id\t{}\n", self.model_id));
        text.push_str(&format!("parent\t{parent_source_generation}\n"));
        text.push_str(&format!(
            "coverage\t{}\t{}\t{}\t{}\t{}\t{}\n",
            self.max_strands,
            self.max_word_length,
            self.normalized_seeds,
            self.distinct_stopping_points,
            self.missing_before,
            self.graph_hits_after
        ));
        text.push_str("edge\tsource_key\ttarget_key\tcc_cost\tu_upper\taudit_sha256\tprogram\n");
        for edge in &self.inserted_edges {
            text.push_str(&format!(
                "edge\t{}\t{}\t{}\t{}\t{}\t{}\n",
                hex(&edge.source_key),
                hex(&edge.target_key),
                edge.cc_cost,
                edge.u_upper_bound,
                hex(&edge.source_audit_sha256),
                hex(&edge.program)
            ));
        }
        text
    }
}

fn with_power_of_two_headroom(observed: u32, minimum: u32) -> Result<u32> {
    observed
        .checked_add(observed / 4)
        .and_then(|value| value.checked_add(1))
        .unwrap_or(u32::MAX)
        .max(minimum)
        .checked_next_power_of_two()
        .ok_or_else(|| "recommended census limit overflow".into())
}

fn distribution(values: impl Iterator<Item = u32>) -> Distribution {
    let mut values: Vec<_> = values.collect();
    values.sort_unstable();
    Distribution {
        p50: percentile(&values, 50),
        p95: percentile(&values, 95),
        p99: percentile(&values, 99),
        max: values.last().copied().unwrap_or(0),
    }
}

fn percentile(values: &[u32], percent: usize) -> u32 {
    if values.is_empty() {
        return 0;
    }
    let rank = (percent * values.len()).div_ceil(100).saturating_sub(1);
    values[rank]
}

fn optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".into())
}

fn word(letters: &[i16]) -> String {
    if letters.is_empty() {
        "-".into()
    } else {
        letters
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{OracleDecision, CONTROLLER_INITIAL_STATE, GRAPH_STOP_OBJECTIVE_RATIO};

    struct ImmediateCcOracle;

    impl PolicyOracle for ImmediateCcOracle {
        fn model_id(&self) -> &str {
            "immediate-cc-test"
        }

        fn objective_ratio(&self) -> u32 {
            GRAPH_STOP_OBJECTIVE_RATIO
        }

        fn controller_initial_state(&self) -> &str {
            CONTROLLER_INITIAL_STATE
        }

        fn first_semantic_decision(
            &mut self,
            _normalized_source: &BraidRepresentation,
            _remaining_policy_plies: u32,
        ) -> Result<OracleDecision> {
            Ok(OracleDecision::PreferredCrossingChange {
                action: SemanticAction::CrossingChange { position: 0 },
                controller_plies: 0,
            })
        }
    }

    #[derive(Default)]
    struct CountingImmediateCcOracle {
        calls: usize,
    }

    impl PolicyOracle for CountingImmediateCcOracle {
        fn model_id(&self) -> &str {
            "counting-immediate-cc-test"
        }

        fn objective_ratio(&self) -> u32 {
            GRAPH_STOP_OBJECTIVE_RATIO
        }

        fn controller_initial_state(&self) -> &str {
            CONTROLLER_INITIAL_STATE
        }

        fn first_semantic_decision(
            &mut self,
            _normalized_source: &BraidRepresentation,
            _remaining_policy_plies: u32,
        ) -> Result<OracleDecision> {
            self.calls += 1;
            Ok(OracleDecision::PreferredCrossingChange {
                action: SemanticAction::CrossingChange { position: 0 },
                controller_plies: 0,
            })
        }
    }

    #[test]
    fn b2_length_ten_census_is_exhaustive_and_normalized() {
        let census = enumerate_natural_braids(2, 10).unwrap();
        assert_eq!(census.raw_knot_inputs, 683);
        assert_eq!(
            census
                .seeds
                .iter()
                .map(|seed| seed.raw_multiplicity)
                .sum::<u64>(),
            683
        );
        assert!(census.seeds.iter().all(|seed| seed
            .representation
            .representation
            .is_normalized()
            .unwrap()));
    }

    #[test]
    fn b3_length_six_census_is_finite_and_normalized() {
        let census = enumerate_natural_braids(3, 6).unwrap();
        assert_eq!(census.max_strands, 3);
        assert_eq!(census.raw_knot_inputs, 2_899);
        assert_eq!(census.seeds.len(), 256);
        assert!(census
            .seeds
            .iter()
            .any(|seed| { seed.representation.representation.strands == 3 }));
        assert!(census.seeds.iter().all(|seed| seed
            .representation
            .representation
            .is_normalized()
            .unwrap()));
    }

    #[test]
    fn recommended_limit_has_headroom() {
        assert_eq!(with_power_of_two_headroom(0, 4).unwrap(), 4);
        assert_eq!(with_power_of_two_headroom(16, 4).unwrap(), 32);
    }

    #[test]
    fn reducer_output_dedup_preserves_every_preprocessing_report() {
        let enumeration = enumerate_natural_braids(2, 10).unwrap();
        let limits = PolicyLimits {
            max_policy_plies: 16,
            max_semantic_moves: 4,
        };
        let mut direct_oracle = CountingImmediateCcOracle::default();
        let direct = enumeration
            .seeds
            .iter()
            .map(|seed| {
                preprocess_to_stopping_point(
                    &seed.representation.representation,
                    &mut direct_oracle,
                    limits,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();

        let mut grouped_oracle = CountingImmediateCcOracle::default();
        let grouped =
            preprocess_seeds_deduplicated(&enumeration.seeds, &mut grouped_oracle, limits).unwrap();
        let grouped_reports = grouped
            .reports
            .into_iter()
            .map(|(report, _)| report)
            .collect::<Vec<_>>();

        assert_eq!(grouped.distinct_reducer_outputs, 5);
        assert_eq!(grouped_reports, direct);
        assert!(grouped_oracle.calls < direct_oracle.calls);
    }

    #[test]
    fn completion_builds_the_full_b2_torus_ladder() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let completion = complete_natural_braids(
            &mut graph,
            &mut ImmediateCcOracle,
            2,
            10,
            PolicyLimits {
                max_policy_plies: 16,
                max_semantic_moves: 4,
            },
        )
        .unwrap();
        assert_eq!(completion.normalized_seeds, 48);
        assert_eq!(completion.distinct_stopping_points, 5);
        assert_eq!(completion.missing_before, 4);
        assert_eq!(completion.inserted_edges.len(), 4);
        assert_eq!(completion.graph_hits_after, 48);
        assert_eq!(graph.node_count(), 5);
        assert_eq!(graph.edge_count(), 4);

        let repeated = complete_natural_braids(
            &mut graph,
            &mut ImmediateCcOracle,
            2,
            10,
            PolicyLimits {
                max_policy_plies: 16,
                max_semantic_moves: 4,
            },
        )
        .unwrap();
        assert_eq!(repeated.missing_before, 0);
        assert!(repeated.inserted_edges.is_empty());
        assert_eq!(graph.edge_count(), 4);
    }
}
