//! Deterministic bounded scramble frontier and replay-checked reverse insertion.

use crate::policy::{
    compile_preprocessing_instructions, preprocess_many_to_stopping_points,
    preprocessing_audit_sha256, PolicyLimits, PolicyOracle, PolicyStopReason,
};
use crate::population::{PopulationGraph, RelaxationOutcome};
use crate::representation::{
    BraidRepresentation, CheckpointedProofProgram, NormalizedRepresentation, ProofInstruction,
    SemanticAction,
};
use crate::{PolicyStopAttestation, RepKey, Result};
use std::collections::{HashSet, VecDeque};

pub const FRONTIER_MANIFEST_VERSION: &str = "unknotdb-frontier-manifest-v1";
pub const FRONTIER_RUN_MANIFEST_VERSION: &str = "unknotdb-frontier-run-manifest-v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrambleLimits {
    pub depth: u16,
    pub max_states: u32,
    pub max_candidates: u32,
    pub max_strands: u16,
    pub max_word_length: u32,
    pub min_cc_cost: u8,
    pub max_cc_cost: u8,
}

impl Default for ScrambleLimits {
    fn default() -> Self {
        Self {
            depth: 3,
            max_states: 512,
            max_candidates: 64,
            max_strands: 3,
            max_word_length: 7,
            min_cc_cost: 1,
            max_cc_cost: 1,
        }
    }
}

impl ScrambleLimits {
    pub fn validate(self) -> Result<()> {
        if self.depth == 0
            || self.max_states == 0
            || self.max_candidates == 0
            || self.max_strands == 0
        {
            return Err("scramble depth/state/candidate/strand bounds must be positive".into());
        }
        if self.min_cc_cost > self.max_cc_cost || self.max_cc_cost > 1 {
            return Err("v0 scramble CC bounds must satisfy 0 <= min <= max <= 1".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrambleCandidate {
    pub actions: Vec<SemanticAction>,
    pub output: BraidRepresentation,
    pub normalized_key: RepKey,
    pub cc_cost: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrambleEnumeration {
    pub candidates: Vec<ScrambleCandidate>,
    pub visited_states: u32,
    pub truncated_by_states: bool,
    pub truncated_by_candidates: bool,
}

impl ScrambleCandidate {
    pub fn program(&self) -> CheckpointedProofProgram {
        CheckpointedProofProgram {
            instructions: self
                .actions
                .iter()
                .copied()
                .map(ProofInstruction::Action)
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrontierDisposition {
    Incomplete(PolicyStopReason),
    DuplicateTarget,
    Inserted { u_upper_bound: u32 },
    Improved { old: u32, new: u32 },
    AcceptedAcs10 { current: u32, proposed: u32 },
    Unchanged { current: u32, proposed: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontierAttempt {
    pub ordinal: u32,
    pub target_key: RepKey,
    pub scramble_cc_cost: u8,
    pub scramble_program: Vec<u8>,
    pub stop_reason: PolicyStopReason,
    pub stopping_key: Option<RepKey>,
    pub preprocessing_audit_sha256: Option<RepKey>,
    pub inverse_edge_program: Option<Vec<u8>>,
    pub disposition: FrontierDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontierBatch {
    pub model_id: String,
    pub policy_limits: PolicyLimits,
    pub scramble_limits: ScrambleLimits,
    pub visited_scramble_states: u32,
    pub truncated_by_states: bool,
    pub truncated_by_candidates: bool,
    pub attempts: Vec<FrontierAttempt>,
    pub inserted: Vec<NormalizedRepresentation>,
}

impl FrontierBatch {
    pub fn manifest_tsv(&self) -> String {
        let mut text = String::new();
        text.push_str(FRONTIER_MANIFEST_VERSION);
        text.push('\n');
        text.push_str(&format!("model_id\t{}\n", self.model_id));
        text.push_str(&format!(
            "policy_limits\t{}\t{}\n",
            self.policy_limits.max_policy_plies, self.policy_limits.max_semantic_moves
        ));
        text.push_str(&format!(
            "scramble_limits\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            self.scramble_limits.depth,
            self.scramble_limits.max_states,
            self.scramble_limits.max_candidates,
            self.scramble_limits.max_strands,
            self.scramble_limits.max_word_length,
            self.scramble_limits.min_cc_cost,
            self.scramble_limits.max_cc_cost
        ));
        text.push_str(&format!(
            "enumeration\t{}\t{}\t{}\n",
            self.visited_scramble_states,
            u8::from(self.truncated_by_states),
            u8::from(self.truncated_by_candidates)
        ));
        text.push_str("attempt\tordinal\ttarget_key\tscramble_cc\tscramble_program\tstop_reason\tstopping_key\taudit_sha256\tinverse_edge_program\tdisposition\n");
        for attempt in &self.attempts {
            text.push_str(&format!(
                "attempt\t{}\t{}\t{}\t{}\t{:?}\t{}\t{}\t{}\t{:?}\n",
                attempt.ordinal,
                hex(&attempt.target_key),
                attempt.scramble_cc_cost,
                hex(&attempt.scramble_program),
                attempt.stop_reason,
                optional_hex(attempt.stopping_key.as_ref()),
                optional_hex(attempt.preprocessing_audit_sha256.as_ref()),
                attempt
                    .inverse_edge_program
                    .as_deref()
                    .map(hex)
                    .unwrap_or_else(|| "-".into()),
                attempt.disposition
            ));
        }
        text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontierRunBatch {
    pub generation: u32,
    pub seed_key: RepKey,
    pub seed_u_upper_bound: u32,
    pub seed_route_rank: u64,
    pub batch: FrontierBatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontierRunManifest {
    pub parent_source_generation: String,
    pub parent_nodes: u64,
    pub parent_edges: u64,
    pub model_id: String,
    pub start_route_rank: u64,
    pub requested_generations: u32,
    pub completed_generations: u32,
    pub batches: Vec<FrontierRunBatch>,
}

impl FrontierRunManifest {
    pub fn manifest_tsv(&self) -> Result<String> {
        if self
            .batches
            .iter()
            .any(|batch| batch.batch.model_id != self.model_id)
        {
            return Err("frontier run mixes policy model IDs".into());
        }
        let mut text = String::new();
        text.push_str(FRONTIER_RUN_MANIFEST_VERSION);
        text.push('\n');
        text.push_str(&format!(
            "parent\t{}\t{}\t{}\n",
            self.parent_source_generation, self.parent_nodes, self.parent_edges
        ));
        text.push_str(&format!("model_id\t{}\n", self.model_id));
        text.push_str(&format!(
            "generations\t{}\t{}\t{}\n",
            self.start_route_rank, self.requested_generations, self.completed_generations
        ));
        text.push_str("seed\tgeneration\tseed_key\tseed_u\tseed_rank\tvisited_states\ttruncated_states\ttruncated_candidates\tattempts\tinserted\n");
        text.push_str("attempt\tgeneration\tseed_key\tordinal\tscramble_cc\tscramble_program\tstop_reason\tstopping_key\taudit_sha256\tinverse_edge_program\tdisposition\n");
        for run_batch in &self.batches {
            let batch = &run_batch.batch;
            let inserted = batch
                .attempts
                .iter()
                .filter(|attempt| {
                    matches!(attempt.disposition, FrontierDisposition::Inserted { .. })
                })
                .count();
            text.push_str(&format!(
                "seed\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                run_batch.generation,
                hex(&run_batch.seed_key),
                run_batch.seed_u_upper_bound,
                run_batch.seed_route_rank,
                batch.visited_scramble_states,
                u8::from(batch.truncated_by_states),
                u8::from(batch.truncated_by_candidates),
                batch.attempts.len(),
                inserted
            ));
            for attempt in &batch.attempts {
                text.push_str(&format!(
                    "attempt\t{}\t{}\t{}\t{}\t{}\t{:?}\t{}\t{}\t{}\t{:?}\n",
                    run_batch.generation,
                    hex(&run_batch.seed_key),
                    attempt.ordinal,
                    attempt.scramble_cc_cost,
                    hex(&attempt.scramble_program),
                    attempt.stop_reason,
                    optional_hex(attempt.stopping_key.as_ref()),
                    optional_hex(attempt.preprocessing_audit_sha256.as_ref()),
                    attempt
                        .inverse_edge_program
                        .as_deref()
                        .map(hex)
                        .unwrap_or_else(|| "-".into()),
                    attempt.disposition
                ));
            }
        }
        Ok(text)
    }
}

#[derive(Clone, Debug)]
struct PendingScramble {
    state: BraidRepresentation,
    actions: Vec<SemanticAction>,
    cc_cost: u8,
}

pub fn enumerate_scrambles(
    source: &BraidRepresentation,
    limits: ScrambleLimits,
) -> Result<Vec<ScrambleCandidate>> {
    Ok(enumerate_scrambles_detailed(source, limits)?.candidates)
}

pub fn enumerate_scrambles_detailed(
    source: &BraidRepresentation,
    limits: ScrambleLimits,
) -> Result<ScrambleEnumeration> {
    limits.validate()?;
    source.validate()?;
    if source.strands > limits.max_strands || source.word.len() > limits.max_word_length as usize {
        return Err("scramble source already exceeds its bounds".into());
    }
    let mut queue = VecDeque::from([PendingScramble {
        state: source.clone(),
        actions: Vec::new(),
        cc_cost: 0,
    }]);
    let mut visited = HashSet::from([source.encode()?]);
    let mut candidate_keys = HashSet::new();
    let mut candidates = Vec::new();
    let mut truncated_by_states = false;
    let mut truncated_by_candidates = false;

    'search: while let Some(pending) = queue.pop_front() {
        if pending.actions.len() == usize::from(limits.depth) {
            if pending.cc_cost >= limits.min_cc_cost
                && pending.cc_cost <= limits.max_cc_cost
                && pending.state.is_knot_closure()?
            {
                let normalized_key = pending.state.normalize()?.key;
                if candidate_keys.insert(normalized_key) {
                    candidates.push(ScrambleCandidate {
                        actions: pending.actions,
                        output: pending.state,
                        normalized_key,
                        cc_cost: pending.cc_cost,
                    });
                    if candidates.len() >= limits.max_candidates as usize {
                        truncated_by_candidates = true;
                        break 'search;
                    }
                }
            }
            continue;
        }

        for action in scramble_actions(&pending.state, pending.cc_cost, limits.max_cc_cost) {
            let Ok(next) = action.apply(&pending.state) else {
                continue;
            };
            if next.strands > limits.max_strands
                || next.word.len() > limits.max_word_length as usize
            {
                continue;
            }
            let encoding = next.encode()?;
            if visited.contains(&encoding) {
                continue;
            }
            if visited.len() >= limits.max_states as usize {
                truncated_by_states = true;
                // The state budget forbids further enqueueing, but candidates
                // already queued at the requested depth remain valid work.
                // Drain them instead of silently returning an empty frontier.
                break;
            }
            visited.insert(encoding);
            let mut actions = pending.actions.clone();
            actions.push(action);
            queue.push_back(PendingScramble {
                state: next,
                actions,
                cc_cost: pending.cc_cost + u8::from(action.is_crossing_change()),
            });
        }
    }
    Ok(ScrambleEnumeration {
        candidates,
        visited_states: visited
            .len()
            .try_into()
            .map_err(|_| "visited scramble-state count does not fit u32")?,
        truncated_by_states,
        truncated_by_candidates,
    })
}

fn scramble_actions(
    state: &BraidRepresentation,
    cc_cost: u8,
    max_cc_cost: u8,
) -> Vec<SemanticAction> {
    let mut actions = Vec::new();
    // Low-branching semantic actions come first so a bounded breadth-first
    // frontier reaches useful CC/stabilization combinations before the much
    // larger insertion family consumes the state budget. This order is part
    // of FRONTIER_MANIFEST_VERSION.
    if cc_cost < max_cc_cost {
        actions.extend(
            (0..state.word.len() as u32)
                .map(|position| SemanticAction::CrossingChange { position }),
        );
    }
    actions.push(SemanticAction::StabilizePositive);
    actions.push(SemanticAction::StabilizeNegative);
    for position in 0..state.word.len() as u32 {
        actions.push(SemanticAction::Commute { position });
    }
    for position in 0..state.word.len() as u32 {
        actions.push(SemanticAction::Braid { position });
    }
    if state.strands >= 2 {
        let positions: Vec<u32> = if state.word.is_empty() {
            vec![0]
        } else {
            (0..state.word.len() as u32).collect()
        };
        let largest = if state.cyclic_band_generators {
            state.strands
        } else {
            state.strands - 1
        };
        for position in positions {
            for generator in 1..=largest {
                for sign in [1, -1] {
                    actions.push(SemanticAction::Insert {
                        position,
                        generator,
                        sign,
                    });
                }
            }
        }
    }
    actions
}

pub fn expand_frontier_seed<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    target: &NormalizedRepresentation,
    oracle: &mut O,
    policy_limits: PolicyLimits,
    scramble_limits: ScrambleLimits,
) -> Result<FrontierBatch> {
    if graph.u_upper_bound(&target.key).is_none() {
        return Err("frontier target is absent from the population graph".into());
    }
    if !target.representation.is_normalized()? {
        return Err("frontier target is not normalized".into());
    }
    let model_id = oracle.model_id().to_owned();
    let enumeration = enumerate_scrambles_detailed(&target.representation, scramble_limits)?;
    let mut attempts = Vec::with_capacity(enumeration.candidates.len());
    let mut inserted = Vec::new();
    let candidates = enumeration.candidates;
    let mut reports = Vec::with_capacity(candidates.len());
    for chunk in candidates.chunks(256) {
        let inputs: Vec<_> = chunk
            .iter()
            .map(|candidate| candidate.output.clone())
            .collect();
        reports.extend(preprocess_many_to_stopping_points(
            &inputs,
            oracle,
            policy_limits,
        )?);
    }

    for (ordinal, (candidate, report)) in candidates.into_iter().zip(reports).enumerate() {
        let ordinal: u32 = ordinal
            .try_into()
            .map_err(|_| "frontier attempt ordinal does not fit u32")?;
        let scramble_program = candidate.program();
        if scramble_program.replay(&target.representation)? != candidate.output {
            return Err("scramble candidate failed exact replay".into());
        }
        let scramble_encoding = scramble_program.encode()?;
        if !report.is_graph_stopping_point() {
            attempts.push(FrontierAttempt {
                ordinal,
                target_key: target.key,
                scramble_cc_cost: candidate.cc_cost,
                scramble_program: scramble_encoding,
                stop_reason: report.stop_reason,
                stopping_key: None,
                preprocessing_audit_sha256: None,
                inverse_edge_program: None,
                disposition: FrontierDisposition::Incomplete(report.stop_reason),
            });
            continue;
        }
        let stopping = report.output.clone();
        let audit_sha256 = preprocessing_audit_sha256(&report)?;
        if stopping.key == target.key {
            attempts.push(FrontierAttempt {
                ordinal,
                target_key: target.key,
                scramble_cc_cost: candidate.cc_cost,
                scramble_program: scramble_encoding,
                stop_reason: report.stop_reason,
                stopping_key: Some(stopping.key),
                preprocessing_audit_sha256: Some(audit_sha256),
                inverse_edge_program: None,
                disposition: FrontierDisposition::DuplicateTarget,
            });
            continue;
        }

        let mut forward_instructions = scramble_program.instructions;
        forward_instructions.extend(compile_preprocessing_instructions(&report)?);
        let forward = CheckpointedProofProgram {
            instructions: forward_instructions,
        };
        if forward.replay(&target.representation)? != stopping.representation {
            return Err("scramble plus preprocessing does not reproduce its stopping point".into());
        }
        let inverse = forward
            .invert_edge(&target.representation)
            .map_err(|error| {
                format!(
                    "frontier candidate {} inverse from {:?} through {:?} to {:?}: {}",
                    ordinal,
                    target.representation,
                    forward.instructions,
                    stopping.representation,
                    error
                )
            })?;
        if inverse.source != stopping.representation || inverse.target != target.representation {
            return Err("inverse frontier edge has mismatched checkpoints".into());
        }
        if inverse.program.cc_cost() != u32::from(candidate.cc_cost) {
            return Err("inverse frontier edge changed scramble CC cost".into());
        }
        let edge_encoding = inverse.program.encode()?;
        let stop_attestation = match report.stop_reason {
            PolicyStopReason::PreferredCrossingChange => {
                PolicyStopAttestation::PreferredCrossingChange {
                    action: report
                        .preferred_crossing_change
                        .ok_or("CC stopping point lacks its action")?
                        .encode_u63()?,
                    audit_sha256,
                }
            }
            PolicyStopReason::TerminalRepresentation => {
                PolicyStopAttestation::Terminal { audit_sha256 }
            }
            _ => unreachable!(),
        };
        let outcome = graph.relax_unknot_edge_deferred_acs10(
            stopping.clone(),
            stop_attestation,
            target.key,
            inverse.program,
            None,
        )?;
        let disposition = match outcome {
            RelaxationOutcome::Inserted { u_upper_bound } => {
                inserted.push(stopping.clone());
                FrontierDisposition::Inserted { u_upper_bound }
            }
            RelaxationOutcome::Improved { old, new } => FrontierDisposition::Improved { old, new },
            RelaxationOutcome::AcceptedAcs10 { current, proposed } => {
                FrontierDisposition::AcceptedAcs10 { current, proposed }
            }
            RelaxationOutcome::Unchanged { current, proposed } => {
                FrontierDisposition::Unchanged { current, proposed }
            }
        };
        attempts.push(FrontierAttempt {
            ordinal,
            target_key: target.key,
            scramble_cc_cost: candidate.cc_cost,
            scramble_program: scramble_encoding,
            stop_reason: report.stop_reason,
            stopping_key: Some(stopping.key),
            preprocessing_audit_sha256: Some(audit_sha256),
            inverse_edge_program: Some(edge_encoding),
            disposition,
        });
    }
    graph.recompute_acs10_routes()?;
    Ok(FrontierBatch {
        model_id,
        policy_limits,
        scramble_limits,
        visited_scramble_states: enumeration.visited_states,
        truncated_by_states: enumeration.truncated_by_states,
        truncated_by_candidates: enumeration.truncated_by_candidates,
        attempts,
        inserted,
    })
}

fn optional_hex(value: Option<&RepKey>) -> String {
    value.map(|key| hex(key)).unwrap_or_else(|| "-".into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_three_unknot_frontier_contains_the_trefoil_orbit() {
        let unknot = BraidRepresentation {
            strands: 1,
            cyclic_band_generators: false,
            word: vec![],
        };
        let trefoil_key = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        }
        .normalize()
        .unwrap()
        .key;
        let candidates = enumerate_scrambles(
            &unknot,
            ScrambleLimits {
                max_states: 2_048,
                max_candidates: 256,
                ..ScrambleLimits::default()
            },
        )
        .unwrap();
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.normalized_key == trefoil_key)
            .expect("trefoil scramble");
        assert_eq!(candidate.cc_cost, 1);
        assert_eq!(candidate.actions.len(), 3);
        assert_eq!(
            candidate.program().replay(&unknot).unwrap(),
            candidate.output
        );
    }

    #[test]
    fn state_cap_still_drains_already_queued_depth_candidates() {
        let trefoil = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        };
        let enumeration = enumerate_scrambles_detailed(
            &trefoil,
            ScrambleLimits {
                depth: 2,
                max_states: 32,
                max_candidates: 32,
                max_strands: 3,
                max_word_length: 7,
                min_cc_cost: 1,
                max_cc_cost: 1,
            },
        )
        .unwrap();
        assert!(enumeration.truncated_by_states);
        assert_eq!(enumeration.visited_states, 32);
        assert!(!enumeration.candidates.is_empty());
        assert!(enumeration
            .candidates
            .iter()
            .all(|candidate| candidate.actions.len() == 2 && candidate.cc_cost == 1));
    }
}
