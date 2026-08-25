//! Bounded, replay-checked policy preprocessing outside the proof trust base.
//!
//! A graph stopping point must be a property of its normalized braid alone.
//! Therefore every policy decision starts from the model's canonical clean
//! controller state. Internal head/memory moves may precede the first semantic
//! proposal, but controller state never becomes part of a graph node.

use crate::reducer::{DeterministicReducer, ReductionReport};
use crate::representation::{
    verify_edge_program, BraidRepresentation, CheckpointedProofProgram, NormalizedRepresentation,
    ProofInstruction, SemanticAction,
};
use crate::{RepKey, Result};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub const POLICY_ADAPTER_VERSION: &str =
    "initial-reducer-clean-controller-top1-self-loop-skip4-mirror-orbit-v3";
pub const LEGACY_POLICY_ADAPTER_VERSIONS: &[&str] =
    &["initial-reducer-clean-controller-top1-mirror-orbit-v2"];

pub fn is_supported_snapshot_adapter(version: &str) -> bool {
    version == POLICY_ADAPTER_VERSION || LEGACY_POLICY_ADAPTER_VERSIONS.contains(&version)
}
pub const GRAPH_STOP_OBJECTIVE_RATIO: u32 = 1000;
pub const CONTROLLER_INITIAL_STATE: &str = "canonical-clean-v0";
const PROTOCOL_HEADER: &str = "UNKNOTDB_POLICY_V0";
const MAX_PROTOCOL_LINE: usize = 8 * 1024;
const MAX_REPRESENTATION_CYCLE_SKIPS: usize = 4;

pub struct PolicyDecisionRequest<'a> {
    pub source: &'a BraidRepresentation,
    pub remaining_policy_plies: u32,
    pub excluded_actions: &'a [SemanticAction],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyLimits {
    /// Applied internal controller actions plus accepted semantic actions.
    /// A preferred CC/PASS is observed but not applied and consumes no ply.
    pub max_policy_plies: u32,
    pub max_semantic_moves: u32,
}

impl Default for PolicyLimits {
    fn default() -> Self {
        Self {
            max_policy_plies: 128,
            max_semantic_moves: 32,
        }
    }
}

impl PolicyLimits {
    fn validate(self) -> Result<()> {
        if self.max_policy_plies == 0 || self.max_semantic_moves == 0 {
            return Err("policy limits must both be positive".into());
        }
        if self.max_semantic_moves > self.max_policy_plies {
            return Err("semantic-move limit cannot exceed the total policy-ply limit".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OracleDecision {
    /// `controller_plies` were already applied inside the model process. The
    /// semantic action is only a proposal until Rust applies it independently.
    Apply {
        action: SemanticAction,
        controller_plies: u32,
    },
    PreferredCrossingChange {
        action: SemanticAction,
        controller_plies: u32,
    },
    PreferredPass {
        controller_plies: u32,
    },
    PolicyPlyLimit {
        controller_plies: u32,
    },
    ControllerCycle {
        controller_plies: u32,
    },
    EnvironmentTerminated {
        controller_plies: u32,
    },
}

impl OracleDecision {
    fn controller_plies(self) -> u32 {
        match self {
            Self::Apply {
                controller_plies, ..
            }
            | Self::PreferredCrossingChange {
                controller_plies, ..
            }
            | Self::PreferredPass { controller_plies }
            | Self::PolicyPlyLimit { controller_plies }
            | Self::ControllerCycle { controller_plies }
            | Self::EnvironmentTerminated { controller_plies } => controller_plies,
        }
    }
}

pub trait PolicyOracle {
    fn model_id(&self) -> &str;
    fn objective_ratio(&self) -> u32;
    fn controller_initial_state(&self) -> &str;
    /// Start from a clean controller and return its first semantic preference.
    fn first_semantic_decision(
        &mut self,
        normalized_source: &BraidRepresentation,
        remaining_policy_plies: u32,
    ) -> Result<OracleDecision>;

    fn first_semantic_decision_excluding(
        &mut self,
        normalized_source: &BraidRepresentation,
        remaining_policy_plies: u32,
        excluded_actions: &[SemanticAction],
    ) -> Result<OracleDecision> {
        if !excluded_actions.is_empty() {
            return Err("policy oracle does not support excluded self-loop actions".into());
        }
        self.first_semantic_decision(normalized_source, remaining_policy_plies)
    }

    /// Batch clean-controller decisions. The default preserves compatibility
    /// with simple/test oracles; inference-backed oracles override this to
    /// share one network forward across independent representations.
    fn first_semantic_decisions(
        &mut self,
        requests: &[(&BraidRepresentation, u32)],
    ) -> Result<Vec<OracleDecision>> {
        requests
            .iter()
            .map(|(source, remaining)| self.first_semantic_decision(source, *remaining))
            .collect()
    }

    fn first_semantic_decisions_excluding(
        &mut self,
        requests: &[PolicyDecisionRequest<'_>],
    ) -> Result<Vec<OracleDecision>> {
        requests
            .iter()
            .map(|request| {
                self.first_semantic_decision_excluding(
                    request.source,
                    request.remaining_policy_plies,
                    request.excluded_actions,
                )
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyStopReason {
    PreferredCrossingChange,
    TerminalRepresentation,
    PreferredPass,
    PolicyPlyLimit,
    SemanticMoveLimit,
    RepresentationCycle,
    ControllerCycle,
    EnvironmentTerminated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyRoundDecision {
    Apply {
        action: SemanticAction,
        controller_plies: u32,
    },
    PreferredCrossingChange {
        action: SemanticAction,
        controller_plies: u32,
    },
    TerminalRepresentation,
    PreferredPass {
        controller_plies: u32,
    },
    PolicyPlyLimit {
        controller_plies: u32,
    },
    SemanticMoveLimit,
    RepresentationCycle {
        proposed_action: SemanticAction,
        controller_plies: u32,
    },
    ControllerCycle {
        controller_plies: u32,
    },
    EnvironmentTerminated {
        controller_plies: u32,
    },
}

impl PolicyRoundDecision {
    fn controller_plies(self) -> u32 {
        match self {
            Self::Apply {
                controller_plies, ..
            }
            | Self::PreferredCrossingChange {
                controller_plies, ..
            }
            | Self::PreferredPass { controller_plies }
            | Self::PolicyPlyLimit { controller_plies }
            | Self::RepresentationCycle {
                controller_plies, ..
            }
            | Self::ControllerCycle { controller_plies }
            | Self::EnvironmentTerminated { controller_plies } => controller_plies,
            Self::TerminalRepresentation | Self::SemanticMoveLimit => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRound {
    /// Canonical representation shown to a fresh policy controller. The
    /// decreasing reducer is used only to produce the first round's source;
    /// later sources are obtained by applying one policy move and normalizing.
    pub source: NormalizedRepresentation,
    pub decision: PolicyRoundDecision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreprocessingReport {
    pub adapter_version: String,
    pub model_id: String,
    pub objective_ratio: u32,
    pub controller_initial_state: String,
    pub limits: PolicyLimits,
    pub input: BraidRepresentation,
    pub initial_reduction: ReductionReport,
    pub rounds: Vec<PolicyRound>,
    pub output: NormalizedRepresentation,
    pub stop_reason: PolicyStopReason,
    pub policy_plies: u32,
    pub semantic_moves: u32,
    pub preferred_crossing_change: Option<SemanticAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyEdgeTrace {
    pub adapter_version: String,
    pub model_id: String,
    pub objective_ratio: u32,
    pub controller_initial_state: String,
    pub limits: PolicyLimits,
    pub source: NormalizedRepresentation,
    pub source_attestation: PreprocessingReport,
    pub crossing_change: SemanticAction,
    pub successor_preprocessing: PreprocessingReport,
    pub target: NormalizedRepresentation,
    pub program: CheckpointedProofProgram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyUnknottingTrace {
    pub initial_preprocessing: PreprocessingReport,
    pub edges: Vec<PolicyEdgeTrace>,
    pub terminal: NormalizedRepresentation,
}

impl PolicyUnknottingTrace {
    pub fn cc_cost(&self) -> u32 {
        self.edges.len() as u32
    }

    pub fn verify(&self) -> Result<()> {
        self.initial_preprocessing.verify()?;
        if !self.initial_preprocessing.is_graph_stopping_point() {
            return Err("unknotting trace starts from incomplete preprocessing".into());
        }
        let mut expected = self.initial_preprocessing.output.clone();
        for (index, edge) in self.edges.iter().enumerate() {
            edge.verify()
                .map_err(|error| format!("unknotting edge {}: {}", index, error))?;
            if edge.source != expected {
                return Err(format!("unknotting edge {} has a discontinuous source", index).into());
            }
            expected = edge.target.clone();
        }
        if expected != self.terminal
            || self.terminal.representation.strands != 1
            || !self.terminal.representation.word.is_empty()
        {
            return Err("unknotting trace does not end at canonical B1 []".into());
        }
        Ok(())
    }
}

impl PolicyEdgeTrace {
    pub fn verify(&self) -> Result<()> {
        self.source_attestation.verify()?;
        self.successor_preprocessing.verify()?;
        for report in [&self.source_attestation, &self.successor_preprocessing] {
            if report.adapter_version != self.adapter_version
                || report.model_id != self.model_id
                || report.objective_ratio != self.objective_ratio
                || report.controller_initial_state != self.controller_initial_state
                || report.limits != self.limits
            {
                return Err("policy edge mixes preprocessing contracts".into());
            }
        }
        if !self.source.representation.is_normalized()? {
            return Err("policy edge source is not normalized".into());
        }
        if self.source_attestation.output != self.source
            || !self.source_attestation.is_graph_stopping_point()
            || !matches!(
                self.source_attestation.rounds.last().unwrap().decision,
                PolicyRoundDecision::PreferredCrossingChange { .. }
            )
        {
            return Err(
                "policy edge source attestation does not end at a CC stopping point".into(),
            );
        }
        if !self.crossing_change.is_crossing_change() {
            return Err("policy edge does not contain a crossing change".into());
        }
        let after_cc = self.crossing_change.apply(&self.source.representation)?;
        if self.successor_preprocessing.input != after_cc {
            return Err("successor preprocessing does not start after the source CC".into());
        }
        if !self.successor_preprocessing.is_graph_stopping_point()
            || self.successor_preprocessing.output != self.target
        {
            return Err("policy edge target is not an admitted graph stopping point".into());
        }
        let encoded = self.program.encode()?;
        verify_edge_program(
            &self.source.encoded,
            &self.target.encoded,
            &encoded,
            CheckpointedProofProgram::VERSION,
            self.crossing_change.encode_u63()?,
            1,
        )?;
        Ok(())
    }
}

impl PreprocessingReport {
    pub fn is_graph_stopping_point(&self) -> bool {
        matches!(
            self.stop_reason,
            PolicyStopReason::PreferredCrossingChange | PolicyStopReason::TerminalRepresentation
        )
    }

    pub fn verify(&self) -> Result<()> {
        self.limits.validate()?;
        if self.adapter_version != POLICY_ADAPTER_VERSION {
            return Err("unsupported policy-adapter version".into());
        }
        if self.model_id.is_empty() {
            return Err("preprocessing report has an empty model id".into());
        }
        if self.objective_ratio != GRAPH_STOP_OBJECTIVE_RATIO {
            return Err("graph stopping points must use the L1000 objective".into());
        }
        if self.controller_initial_state != CONTROLLER_INITIAL_STATE {
            return Err("unsupported policy controller initial-state rule".into());
        }
        if self.rounds.is_empty() {
            return Err("preprocessing report has no reducer/policy round".into());
        }

        if self.initial_reduction.input != self.input {
            return Err("initial reducer input does not match preprocessing input".into());
        }
        self.initial_reduction.verify()?;

        let mut expected_source = self.initial_reduction.output.clone();
        let mut seen = HashSet::<RepKey>::new();
        let mut recomputed_plies = 0_u32;
        let mut recomputed_semantic = 0_u32;
        let mut terminal_reason = None;
        let mut preferred_cc = None;

        for (index, round) in self.rounds.iter().enumerate() {
            if terminal_reason.is_some() {
                return Err("preprocessing report continues after a terminal round".into());
            }
            if round.source != expected_source {
                return Err(format!("policy round {} has a discontinuous source", index).into());
            }
            if !round.source.representation.is_normalized()? {
                return Err(format!("policy round {} source is not normalized", index).into());
            }
            let source = &round.source.representation;
            if !seen.insert(round.source.key) {
                return Err(
                    format!("policy round {} repeats a stopping representation", index).into(),
                );
            }
            let controller_plies = round.decision.controller_plies();
            recomputed_plies = recomputed_plies
                .checked_add(controller_plies)
                .ok_or("policy ply count overflow")?;

            match round.decision {
                PolicyRoundDecision::Apply { action, .. } => {
                    if action.is_crossing_change() || matches!(action, SemanticAction::Pass) {
                        return Err("policy apply decision is not a zero-CC state change".into());
                    }
                    recomputed_plies = recomputed_plies
                        .checked_add(1)
                        .ok_or("policy ply count overflow")?;
                    recomputed_semantic += 1;
                    expected_source = action.apply(source)?.normalize()?;
                }
                PolicyRoundDecision::PreferredCrossingChange { action, .. } => {
                    if !action.is_crossing_change() {
                        return Err("preferred crossing-change decision is not CC".into());
                    }
                    action.apply(source)?;
                    preferred_cc = Some(action);
                    terminal_reason = Some(PolicyStopReason::PreferredCrossingChange);
                }
                PolicyRoundDecision::TerminalRepresentation => {
                    if !is_terminal_representation(source) {
                        return Err("terminal policy round is not the canonical unknot".into());
                    }
                    terminal_reason = Some(PolicyStopReason::TerminalRepresentation);
                }
                PolicyRoundDecision::PreferredPass { .. } => {
                    terminal_reason = Some(PolicyStopReason::PreferredPass);
                }
                PolicyRoundDecision::PolicyPlyLimit { .. } => {
                    terminal_reason = Some(PolicyStopReason::PolicyPlyLimit);
                }
                PolicyRoundDecision::SemanticMoveLimit => {
                    terminal_reason = Some(PolicyStopReason::SemanticMoveLimit);
                }
                PolicyRoundDecision::RepresentationCycle {
                    proposed_action, ..
                } => {
                    if proposed_action.is_crossing_change()
                        || matches!(proposed_action, SemanticAction::Pass)
                    {
                        return Err("cycle proposal is not a zero-CC state change".into());
                    }
                    proposed_action.apply(source)?;
                    terminal_reason = Some(PolicyStopReason::RepresentationCycle);
                }
                PolicyRoundDecision::ControllerCycle { .. } => {
                    terminal_reason = Some(PolicyStopReason::ControllerCycle);
                }
                PolicyRoundDecision::EnvironmentTerminated { .. } => {
                    terminal_reason = Some(PolicyStopReason::EnvironmentTerminated);
                }
            }
        }

        let reason = terminal_reason.ok_or("preprocessing report has no terminal decision")?;
        if reason != self.stop_reason {
            return Err("preprocessing stop reason does not match its last round".into());
        }
        if recomputed_plies != self.policy_plies || recomputed_semantic != self.semantic_moves {
            return Err("preprocessing ply/move totals do not match replay".into());
        }
        if self.policy_plies > self.limits.max_policy_plies
            || self.semantic_moves > self.limits.max_semantic_moves
        {
            return Err("preprocessing route exceeds its declared limits".into());
        }
        if preferred_cc != self.preferred_crossing_change {
            return Err("preferred CC does not match the terminal policy decision".into());
        }
        if self.output != self.rounds.last().unwrap().source {
            return Err("preprocessing output is not the last policy checkpoint".into());
        }
        if self.stop_reason == PolicyStopReason::PreferredCrossingChange
            && self.preferred_crossing_change.is_none()
        {
            return Err("nonterminal graph stopping point lacks its preferred CC".into());
        }
        Ok(())
    }
}

pub fn preprocess_to_stopping_point<O: PolicyOracle>(
    input: &BraidRepresentation,
    oracle: &mut O,
    limits: PolicyLimits,
) -> Result<PreprocessingReport> {
    limits.validate()?;
    if oracle.objective_ratio() != GRAPH_STOP_OBJECTIVE_RATIO {
        return Err("policy oracle is not configured for L1000".into());
    }
    if oracle.controller_initial_state() != CONTROLLER_INITIAL_STATE {
        return Err("policy oracle does not use the canonical clean controller".into());
    }
    let model_id = oracle.model_id().to_owned();
    if model_id.is_empty() {
        return Err("policy oracle supplied an empty model id".into());
    }

    let mut rounds = Vec::new();
    let initial_reduction = DeterministicReducer::reduce(input)?;
    let mut current = initial_reduction.output.clone();
    let mut seen = HashSet::<RepKey>::new();
    seen.insert(current.key);
    let mut policy_plies = 0_u32;
    let mut semantic_moves = 0_u32;
    let mut preferred_crossing_change = None;
    let mut excluded_actions = Vec::new();

    let stop_reason = loop {
        let source = &current.representation;
        if is_terminal_representation(source) {
            rounds.push(PolicyRound {
                source: current,
                decision: PolicyRoundDecision::TerminalRepresentation,
            });
            break PolicyStopReason::TerminalRepresentation;
        }
        if semantic_moves >= limits.max_semantic_moves {
            rounds.push(PolicyRound {
                source: current,
                decision: PolicyRoundDecision::SemanticMoveLimit,
            });
            break PolicyStopReason::SemanticMoveLimit;
        }
        if policy_plies >= limits.max_policy_plies {
            rounds.push(PolicyRound {
                source: current,
                decision: PolicyRoundDecision::PolicyPlyLimit {
                    controller_plies: 0,
                },
            });
            break PolicyStopReason::PolicyPlyLimit;
        }

        let remaining = limits.max_policy_plies - policy_plies;
        let decision =
            oracle.first_semantic_decision_excluding(source, remaining, &excluded_actions)?;
        let controller_plies = decision.controller_plies();
        if controller_plies > remaining {
            return Err(format!(
                "policy oracle consumed {} controller plies with only {} remaining",
                controller_plies, remaining
            )
            .into());
        }

        match decision {
            OracleDecision::Apply { action, .. } => {
                if action.is_crossing_change() || matches!(action, SemanticAction::Pass) {
                    return Err("policy oracle returned CC/PASS as an apply decision".into());
                }
                if policy_plies + controller_plies >= limits.max_policy_plies {
                    policy_plies += controller_plies;
                    rounds.push(PolicyRound {
                        source: current,
                        decision: PolicyRoundDecision::PolicyPlyLimit { controller_plies },
                    });
                    break PolicyStopReason::PolicyPlyLimit;
                }
                let raw_next = action.apply(source)?;
                let next = raw_next.normalize()?;
                if seen.contains(&next.key) {
                    if excluded_actions.len() < MAX_REPRESENTATION_CYCLE_SKIPS
                        && !excluded_actions.contains(&action)
                    {
                        excluded_actions.push(action);
                        continue;
                    }
                    policy_plies += controller_plies;
                    rounds.push(PolicyRound {
                        source: current,
                        decision: PolicyRoundDecision::RepresentationCycle {
                            proposed_action: action,
                            controller_plies,
                        },
                    });
                    break PolicyStopReason::RepresentationCycle;
                }
                policy_plies += controller_plies;
                policy_plies += 1;
                semantic_moves += 1;
                rounds.push(PolicyRound {
                    source: current,
                    decision: PolicyRoundDecision::Apply {
                        action,
                        controller_plies,
                    },
                });
                seen.insert(next.key);
                current = next;
                excluded_actions.clear();
            }
            OracleDecision::PreferredCrossingChange { action, .. } => {
                policy_plies += controller_plies;
                if !action.is_crossing_change() {
                    return Err("policy oracle's preferred-CC decision is not a CC action".into());
                }
                action.apply(source)?;
                preferred_crossing_change = Some(action);
                rounds.push(PolicyRound {
                    source: current,
                    decision: PolicyRoundDecision::PreferredCrossingChange {
                        action,
                        controller_plies,
                    },
                });
                break PolicyStopReason::PreferredCrossingChange;
            }
            OracleDecision::PreferredPass { .. } => {
                policy_plies += controller_plies;
                rounds.push(PolicyRound {
                    source: current,
                    decision: PolicyRoundDecision::PreferredPass { controller_plies },
                });
                break PolicyStopReason::PreferredPass;
            }
            OracleDecision::PolicyPlyLimit { .. } => {
                policy_plies += controller_plies;
                rounds.push(PolicyRound {
                    source: current,
                    decision: PolicyRoundDecision::PolicyPlyLimit { controller_plies },
                });
                break PolicyStopReason::PolicyPlyLimit;
            }
            OracleDecision::ControllerCycle { .. } => {
                policy_plies += controller_plies;
                rounds.push(PolicyRound {
                    source: current,
                    decision: PolicyRoundDecision::ControllerCycle { controller_plies },
                });
                break PolicyStopReason::ControllerCycle;
            }
            OracleDecision::EnvironmentTerminated { .. } => {
                policy_plies += controller_plies;
                rounds.push(PolicyRound {
                    source: current,
                    decision: PolicyRoundDecision::EnvironmentTerminated { controller_plies },
                });
                break PolicyStopReason::EnvironmentTerminated;
            }
        }
    };

    let output = rounds.last().unwrap().source.clone();
    let report = PreprocessingReport {
        adapter_version: POLICY_ADAPTER_VERSION.into(),
        model_id,
        objective_ratio: GRAPH_STOP_OBJECTIVE_RATIO,
        controller_initial_state: CONTROLLER_INITIAL_STATE.into(),
        limits,
        input: input.clone(),
        initial_reduction,
        rounds,
        output,
        stop_reason,
        policy_plies,
        semantic_moves,
        preferred_crossing_change,
    };
    report.verify()?;
    Ok(report)
}

/// Preprocess independent inputs in policy rounds, batching every set of
/// clean-controller decisions while preserving the scalar adapter semantics.
pub fn preprocess_many_to_stopping_points<O: PolicyOracle>(
    inputs: &[BraidRepresentation],
    oracle: &mut O,
    limits: PolicyLimits,
) -> Result<Vec<PreprocessingReport>> {
    if inputs.len() <= 1 {
        return inputs
            .iter()
            .map(|input| preprocess_to_stopping_point(input, oracle, limits))
            .collect();
    }
    limits.validate()?;
    if oracle.objective_ratio() != GRAPH_STOP_OBJECTIVE_RATIO
        || oracle.controller_initial_state() != CONTROLLER_INITIAL_STATE
        || oracle.model_id().is_empty()
    {
        return Err("policy oracle does not satisfy the graph preprocessing contract".into());
    }
    let model_id = oracle.model_id().to_owned();

    struct State {
        input: BraidRepresentation,
        initial_reduction: ReductionReport,
        rounds: Vec<PolicyRound>,
        current: NormalizedRepresentation,
        seen: HashSet<RepKey>,
        policy_plies: u32,
        semantic_moves: u32,
        preferred_crossing_change: Option<SemanticAction>,
        stop_reason: Option<PolicyStopReason>,
        excluded_actions: Vec<SemanticAction>,
    }

    let mut states = Vec::with_capacity(inputs.len());
    for input in inputs {
        let initial_reduction = DeterministicReducer::reduce(input)?;
        let current = initial_reduction.output.clone();
        states.push(State {
            input: input.clone(),
            initial_reduction,
            rounds: Vec::new(),
            seen: HashSet::from([current.key]),
            current,
            policy_plies: 0,
            semantic_moves: 0,
            preferred_crossing_change: None,
            stop_reason: None,
            excluded_actions: Vec::new(),
        });
    }

    while states.iter().any(|state| state.stop_reason.is_none()) {
        let mut active = Vec::new();
        for (index, state) in states.iter_mut().enumerate() {
            if state.stop_reason.is_some() {
                continue;
            }
            let local_stop = if is_terminal_representation(&state.current.representation) {
                Some((
                    PolicyRoundDecision::TerminalRepresentation,
                    PolicyStopReason::TerminalRepresentation,
                ))
            } else if state.semantic_moves >= limits.max_semantic_moves {
                Some((
                    PolicyRoundDecision::SemanticMoveLimit,
                    PolicyStopReason::SemanticMoveLimit,
                ))
            } else if state.policy_plies >= limits.max_policy_plies {
                Some((
                    PolicyRoundDecision::PolicyPlyLimit {
                        controller_plies: 0,
                    },
                    PolicyStopReason::PolicyPlyLimit,
                ))
            } else {
                None
            };
            if let Some((decision, reason)) = local_stop {
                state.rounds.push(PolicyRound {
                    source: state.current.clone(),
                    decision,
                });
                state.stop_reason = Some(reason);
            } else {
                active.push(index);
            }
        }
        if active.is_empty() {
            break;
        }

        let requests: Vec<_> = active
            .iter()
            .map(|index| {
                let state = &states[*index];
                PolicyDecisionRequest {
                    source: &state.current.representation,
                    remaining_policy_plies: limits.max_policy_plies - state.policy_plies,
                    excluded_actions: &state.excluded_actions,
                }
            })
            .collect();
        let decisions = oracle.first_semantic_decisions_excluding(&requests)?;
        if decisions.len() != active.len() {
            return Err("policy oracle returned the wrong batch length".into());
        }

        for (index, decision) in active.into_iter().zip(decisions) {
            let state = &mut states[index];
            let remaining = limits.max_policy_plies - state.policy_plies;
            let controller_plies = decision.controller_plies();
            if controller_plies > remaining {
                return Err("batch policy oracle exceeded the remaining ply budget".into());
            }
            let source = state.current.clone();
            match decision {
                OracleDecision::Apply { action, .. } => {
                    if action.is_crossing_change() || matches!(action, SemanticAction::Pass) {
                        return Err("policy oracle returned CC/PASS as an apply decision".into());
                    }
                    if state.policy_plies + controller_plies >= limits.max_policy_plies {
                        state.policy_plies += controller_plies;
                        state.rounds.push(PolicyRound {
                            source,
                            decision: PolicyRoundDecision::PolicyPlyLimit { controller_plies },
                        });
                        state.stop_reason = Some(PolicyStopReason::PolicyPlyLimit);
                        continue;
                    }
                    let next = action.apply(&source.representation)?.normalize()?;
                    if state.seen.contains(&next.key) {
                        if state.excluded_actions.len() < MAX_REPRESENTATION_CYCLE_SKIPS
                            && !state.excluded_actions.contains(&action)
                        {
                            state.excluded_actions.push(action);
                            continue;
                        }
                        state.policy_plies += controller_plies;
                        state.rounds.push(PolicyRound {
                            source,
                            decision: PolicyRoundDecision::RepresentationCycle {
                                proposed_action: action,
                                controller_plies,
                            },
                        });
                        state.stop_reason = Some(PolicyStopReason::RepresentationCycle);
                        continue;
                    }
                    state.policy_plies += controller_plies;
                    state.policy_plies += 1;
                    state.semantic_moves += 1;
                    state.rounds.push(PolicyRound {
                        source,
                        decision: PolicyRoundDecision::Apply {
                            action,
                            controller_plies,
                        },
                    });
                    state.seen.insert(next.key);
                    state.current = next;
                    state.excluded_actions.clear();
                }
                OracleDecision::PreferredCrossingChange { action, .. } => {
                    state.policy_plies += controller_plies;
                    if !action.is_crossing_change() {
                        return Err("policy oracle's preferred-CC decision is not CC".into());
                    }
                    action.apply(&source.representation)?;
                    state.preferred_crossing_change = Some(action);
                    state.rounds.push(PolicyRound {
                        source,
                        decision: PolicyRoundDecision::PreferredCrossingChange {
                            action,
                            controller_plies,
                        },
                    });
                    state.stop_reason = Some(PolicyStopReason::PreferredCrossingChange);
                }
                OracleDecision::PreferredPass { .. } => {
                    state.policy_plies += controller_plies;
                    state.rounds.push(PolicyRound {
                        source,
                        decision: PolicyRoundDecision::PreferredPass { controller_plies },
                    });
                    state.stop_reason = Some(PolicyStopReason::PreferredPass);
                }
                OracleDecision::PolicyPlyLimit { .. } => {
                    state.policy_plies += controller_plies;
                    state.rounds.push(PolicyRound {
                        source,
                        decision: PolicyRoundDecision::PolicyPlyLimit { controller_plies },
                    });
                    state.stop_reason = Some(PolicyStopReason::PolicyPlyLimit);
                }
                OracleDecision::ControllerCycle { .. } => {
                    state.policy_plies += controller_plies;
                    state.rounds.push(PolicyRound {
                        source,
                        decision: PolicyRoundDecision::ControllerCycle { controller_plies },
                    });
                    state.stop_reason = Some(PolicyStopReason::ControllerCycle);
                }
                OracleDecision::EnvironmentTerminated { .. } => {
                    state.policy_plies += controller_plies;
                    state.rounds.push(PolicyRound {
                        source,
                        decision: PolicyRoundDecision::EnvironmentTerminated { controller_plies },
                    });
                    state.stop_reason = Some(PolicyStopReason::EnvironmentTerminated);
                }
            }
        }
    }

    states
        .into_iter()
        .map(|state| {
            let output = state
                .rounds
                .last()
                .ok_or("batched preprocessing produced no policy rounds")?
                .source
                .clone();
            let report = PreprocessingReport {
                adapter_version: POLICY_ADAPTER_VERSION.into(),
                model_id: model_id.clone(),
                objective_ratio: GRAPH_STOP_OBJECTIVE_RATIO,
                controller_initial_state: CONTROLLER_INITIAL_STATE.into(),
                limits,
                input: state.input,
                initial_reduction: state.initial_reduction,
                rounds: state.rounds,
                output,
                stop_reason: state
                    .stop_reason
                    .ok_or("batched preprocessing did not stop")?,
                policy_plies: state.policy_plies,
                semantic_moves: state.semantic_moves,
                preferred_crossing_change: state.preferred_crossing_change,
            };
            report.verify()?;
            Ok(report)
        })
        .collect()
}

/// Trace one weighted proof macro: exactly one source CC followed by the
/// independently replayed zero-CC route to the next admitted stopping point.
pub fn trace_policy_edge<O: PolicyOracle>(
    source: &BraidRepresentation,
    oracle: &mut O,
    limits: PolicyLimits,
) -> Result<PolicyEdgeTrace> {
    if !source.is_normalized()? {
        return Err("graph edge source must already be normalized".into());
    }
    let source_attestation = preprocess_to_stopping_point(source, oracle, limits)?;
    if source_attestation.output.representation != *source {
        return Err("source is not an immediate policy stopping point".into());
    }
    trace_policy_edge_from_attestation(source_attestation, oracle)
}

pub fn trace_policy_edge_from_attestation<O: PolicyOracle>(
    source_attestation: PreprocessingReport,
    oracle: &mut O,
) -> Result<PolicyEdgeTrace> {
    source_attestation.verify()?;
    if !source_attestation.is_graph_stopping_point() {
        return Err("source attestation is not a graph stopping point".into());
    }
    let crossing_change = source_attestation
        .preferred_crossing_change
        .ok_or("terminal B1 [] has no outgoing policy edge")?;
    trace_crossing_change_edge_from_attestation(source_attestation, crossing_change, oracle)
}

/// Trace one verified CC macro from a policy stopping point. The graph builder
/// normally uses the policy-preferred CC, but may select another legal CC to
/// escape a bounded greedy cycle. The source attestation still proves that the
/// representation itself is an admitted stopping point.
pub fn trace_crossing_change_edge_from_attestation<O: PolicyOracle>(
    source_attestation: PreprocessingReport,
    crossing_change: SemanticAction,
    oracle: &mut O,
) -> Result<PolicyEdgeTrace> {
    source_attestation.verify()?;
    if !source_attestation.is_graph_stopping_point() {
        return Err("source attestation is not a graph stopping point".into());
    }
    if !crossing_change.is_crossing_change() {
        return Err("graph edge action is not a crossing change".into());
    }
    let source = source_attestation.output.clone();
    let after_cc = crossing_change.apply(&source.representation)?;
    let limits = source_attestation.limits;
    let successor_preprocessing = preprocess_to_stopping_point(&after_cc, oracle, limits)?;
    if !successor_preprocessing.is_graph_stopping_point() {
        return Err(format!(
            "successor preprocessing did not reach a graph stopping point: {:?}",
            successor_preprocessing.stop_reason
        )
        .into());
    }

    let mut instructions = vec![ProofInstruction::Action(crossing_change)];
    instructions.extend(compile_preprocessing_instructions(
        &successor_preprocessing,
    )?);
    let target = successor_preprocessing.output.clone();
    let trace = PolicyEdgeTrace {
        adapter_version: source_attestation.adapter_version.clone(),
        model_id: source_attestation.model_id.clone(),
        objective_ratio: source_attestation.objective_ratio,
        controller_initial_state: source_attestation.controller_initial_state.clone(),
        limits,
        source,
        source_attestation,
        crossing_change,
        successor_preprocessing,
        target,
        program: CheckpointedProofProgram { instructions },
    };
    trace.verify()?;
    Ok(trace)
}

/// Follow policy-selected CC macros until the canonical unknot. The bound is
/// explicit because the heuristic can cycle or fail to unknot an input.
pub fn trace_policy_to_unknot<O: PolicyOracle>(
    input: &BraidRepresentation,
    oracle: &mut O,
    limits: PolicyLimits,
    max_crossing_changes: u32,
) -> Result<PolicyUnknottingTrace> {
    if max_crossing_changes == 0 {
        return Err("max_crossing_changes must be positive".into());
    }
    let initial_preprocessing = preprocess_to_stopping_point(input, oracle, limits)?;
    if !initial_preprocessing.is_graph_stopping_point() {
        return Err(format!(
            "initial preprocessing did not reach a graph stopping point: {:?}",
            initial_preprocessing.stop_reason
        )
        .into());
    }
    let mut attestation = initial_preprocessing.clone();
    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(attestation.output.key);

    while attestation.stop_reason != PolicyStopReason::TerminalRepresentation {
        if edges.len() >= max_crossing_changes as usize {
            return Err(format!(
                "policy unknotting trace exceeded {} crossing changes",
                max_crossing_changes
            )
            .into());
        }
        let edge = trace_policy_edge_from_attestation(attestation, oracle)?;
        if !seen.insert(edge.target.key) {
            return Err("policy unknotting trace repeats a stopping-point key".into());
        }
        attestation = edge.successor_preprocessing.clone();
        edges.push(edge);
    }
    let terminal = attestation.output;
    let trace = PolicyUnknottingTrace {
        initial_preprocessing,
        edges,
        terminal,
    };
    trace.verify()?;
    Ok(trace)
}

pub fn compile_preprocessing_instructions(
    report: &PreprocessingReport,
) -> Result<Vec<ProofInstruction>> {
    report.verify()?;
    if !report.is_graph_stopping_point() {
        return Err("cannot compile an incomplete preprocessing route as an edge".into());
    }
    let mut instructions = Vec::new();
    push_normalization(
        &mut instructions,
        report.initial_reduction.input_normalization,
    );
    instructions.extend(
        report
            .initial_reduction
            .program
            .actions
            .iter()
            .copied()
            .map(ProofInstruction::Action),
    );
    push_normalization(&mut instructions, report.initial_reduction.output.witness);
    for (index, round) in report.rounds.iter().enumerate() {
        match round.decision {
            PolicyRoundDecision::Apply { action, .. } => {
                if action.is_crossing_change() || matches!(action, SemanticAction::Pass) {
                    return Err("compiled zero-CC route contains a non-zero-cost action".into());
                }
                instructions.push(ProofInstruction::Action(action));
                let next = report
                    .rounds
                    .get(index + 1)
                    .ok_or("compiled preprocessing route ends after an applied policy move")?;
                push_normalization(&mut instructions, next.source.witness);
            }
            PolicyRoundDecision::PreferredCrossingChange { .. }
            | PolicyRoundDecision::TerminalRepresentation => {}
            _ => {
                return Err("compiled preprocessing route has an incomplete terminal round".into())
            }
        }
    }
    if instructions.iter().any(
        |instruction| matches!(instruction, ProofInstruction::Action(action) if action.is_crossing_change()),
    ) {
        return Err("compiled preprocessing suffix contains a crossing change".into());
    }
    Ok(instructions)
}

/// Stable content hash for the replay-relevant preprocessing audit. This is
/// intentionally independent of Rust's `Debug` output.
pub fn preprocessing_audit_sha256(report: &PreprocessingReport) -> Result<RepKey> {
    report.verify()?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"UNKNOTDB_PREPROCESSING_AUDIT_V1\0");
    push_audit_string(&mut bytes, &report.adapter_version)?;
    push_audit_string(&mut bytes, &report.model_id)?;
    push_audit_string(&mut bytes, &report.controller_initial_state)?;
    bytes.extend_from_slice(&report.objective_ratio.to_le_bytes());
    bytes.extend_from_slice(&report.limits.max_policy_plies.to_le_bytes());
    bytes.extend_from_slice(&report.limits.max_semantic_moves.to_le_bytes());
    push_audit_blob(&mut bytes, &report.input.encode()?)?;
    push_audit_blob(&mut bytes, &report.output.encoded)?;
    bytes.extend_from_slice(&report.policy_plies.to_le_bytes());
    bytes.extend_from_slice(&report.semantic_moves.to_le_bytes());
    bytes.push(stop_reason_code(report.stop_reason));
    bytes.extend_from_slice(
        &report
            .preferred_crossing_change
            .map(SemanticAction::encode_u63)
            .transpose()?
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    let compiled = CheckpointedProofProgram {
        instructions: compile_preprocessing_instructions(report)?,
    };
    push_audit_blob(&mut bytes, &compiled.encode()?)?;
    let round_count: u32 = report
        .rounds
        .len()
        .try_into()
        .map_err(|_| "too many preprocessing audit rounds")?;
    bytes.extend_from_slice(&round_count.to_le_bytes());
    push_audit_blob(&mut bytes, &report.initial_reduction.input.encode()?)?;
    push_audit_blob(&mut bytes, &report.initial_reduction.output.encoded)?;
    for round in &report.rounds {
        push_audit_blob(&mut bytes, &round.source.encoded)?;
        encode_round_decision(&mut bytes, round.decision)?;
    }
    sha256_digest(&bytes)
}

fn push_audit_string(bytes: &mut Vec<u8>, value: &str) -> Result<()> {
    push_audit_blob(bytes, value.as_bytes())
}

fn push_audit_blob(bytes: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let len: u32 = value
        .len()
        .try_into()
        .map_err(|_| "preprocessing audit field is too large")?;
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

fn encode_round_decision(bytes: &mut Vec<u8>, decision: PolicyRoundDecision) -> Result<()> {
    let (kind, action, plies) = match decision {
        PolicyRoundDecision::Apply {
            action,
            controller_plies,
        } => (0, Some(action), controller_plies),
        PolicyRoundDecision::PreferredCrossingChange {
            action,
            controller_plies,
        } => (1, Some(action), controller_plies),
        PolicyRoundDecision::TerminalRepresentation => (2, None, 0),
        PolicyRoundDecision::PreferredPass { controller_plies } => (3, None, controller_plies),
        PolicyRoundDecision::PolicyPlyLimit { controller_plies } => (4, None, controller_plies),
        PolicyRoundDecision::SemanticMoveLimit => (5, None, 0),
        PolicyRoundDecision::RepresentationCycle {
            proposed_action,
            controller_plies,
        } => (6, Some(proposed_action), controller_plies),
        PolicyRoundDecision::ControllerCycle { controller_plies } => (7, None, controller_plies),
        PolicyRoundDecision::EnvironmentTerminated { controller_plies } => {
            (8, None, controller_plies)
        }
    };
    bytes.push(kind);
    bytes.extend_from_slice(&plies.to_le_bytes());
    bytes.extend_from_slice(
        &action
            .map(SemanticAction::encode_u63)
            .transpose()?
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    Ok(())
}

fn stop_reason_code(reason: PolicyStopReason) -> u8 {
    match reason {
        PolicyStopReason::PreferredCrossingChange => 0,
        PolicyStopReason::TerminalRepresentation => 1,
        PolicyStopReason::PreferredPass => 2,
        PolicyStopReason::PolicyPlyLimit => 3,
        PolicyStopReason::SemanticMoveLimit => 4,
        PolicyStopReason::RepresentationCycle => 5,
        PolicyStopReason::ControllerCycle => 6,
        PolicyStopReason::EnvironmentTerminated => 7,
    }
}

fn sha256_digest(bytes: &[u8]) -> Result<RepKey> {
    let encoded = unknotdb::util::sha256_hex(bytes);
    let mut digest = [0_u8; 32];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(digest)
}

fn push_normalization(
    instructions: &mut Vec<ProofInstruction>,
    witness: crate::representation::NormalizationWitness,
) {
    if witness.mirrored {
        instructions.push(ProofInstruction::MirrorOrbit);
    }
    if witness.rotate_word_left != 0 {
        instructions.push(ProofInstruction::NormalizeOrigin {
            rotate_word_left: witness.rotate_word_left,
        });
    }
}

fn is_terminal_representation(representation: &BraidRepresentation) -> bool {
    representation.strands == 1 && representation.word.is_empty()
}

/// Line-protocol client. The external process owns Torch/JAX and controller
/// internals; Rust owns objective/version checks, bounds and semantic replay.
pub struct ExternalPolicyOracle {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
    model_id: String,
    objective_ratio: u32,
    controller_initial_state: String,
}

impl ExternalPolicyOracle {
    pub fn spawn(program: &Path, args: &[String]) -> Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = BufWriter::new(child.stdin.take().ok_or("policy process has no stdin")?);
        let mut output = BufReader::new(child.stdout.take().ok_or("policy process has no stdout")?);
        let header = read_protocol_line(&mut output)?;
        let fields: Vec<_> = header.split_whitespace().collect();
        let [protocol, model_id, objective_ratio, controller_initial_state] = fields.as_slice()
        else {
            return Err(format!("bad policy-process header `{header}`").into());
        };
        if *protocol != PROTOCOL_HEADER || model_id.is_empty() {
            return Err(format!("bad policy-process header `{header}`").into());
        }
        Ok(Self {
            child,
            input,
            output,
            model_id: (*model_id).into(),
            objective_ratio: parse_u32(objective_ratio, "objective ratio")?,
            controller_initial_state: (*controller_initial_state).into(),
        })
    }

    fn request(&mut self, line: &str) -> Result<String> {
        writeln!(self.input, "{line}")?;
        self.input.flush()?;
        let response = read_protocol_line(&mut self.output)?;
        if let Some(error) = response.strip_prefix("ERROR ") {
            return Err(format!("policy process: {error}").into());
        }
        Ok(response)
    }
}

impl PolicyOracle for ExternalPolicyOracle {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn objective_ratio(&self) -> u32 {
        self.objective_ratio
    }

    fn controller_initial_state(&self) -> &str {
        &self.controller_initial_state
    }

    fn first_semantic_decision(
        &mut self,
        normalized_source: &BraidRepresentation,
        remaining_policy_plies: u32,
    ) -> Result<OracleDecision> {
        self.first_semantic_decision_excluding(normalized_source, remaining_policy_plies, &[])
    }

    fn first_semantic_decision_excluding(
        &mut self,
        normalized_source: &BraidRepresentation,
        remaining_policy_plies: u32,
        excluded_actions: &[SemanticAction],
    ) -> Result<OracleDecision> {
        if !normalized_source.is_normalized()? {
            return Err("external policy request is not normalized".into());
        }
        let response = self.request(&format!(
            "DECIDE {} {} {}",
            encode_hex(&normalized_source.encode()?),
            remaining_policy_plies,
            encode_excluded_actions(excluded_actions)?,
        ))?;
        parse_oracle_decision(&response)
    }

    fn first_semantic_decisions(
        &mut self,
        requests: &[(&BraidRepresentation, u32)],
    ) -> Result<Vec<OracleDecision>> {
        let requests: Vec<_> = requests
            .iter()
            .map(|(source, remaining_policy_plies)| PolicyDecisionRequest {
                source,
                remaining_policy_plies: *remaining_policy_plies,
                excluded_actions: &[],
            })
            .collect();
        self.first_semantic_decisions_excluding(&requests)
    }

    fn first_semantic_decisions_excluding(
        &mut self,
        requests: &[PolicyDecisionRequest<'_>],
    ) -> Result<Vec<OracleDecision>> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        writeln!(self.input, "DECIDE_BATCH {}", requests.len())?;
        for request in requests {
            if !request.source.is_normalized()? {
                return Err("external batch policy request is not normalized".into());
            }
            writeln!(
                self.input,
                "{} {} {}",
                encode_hex(&request.source.encode()?),
                request.remaining_policy_plies,
                encode_excluded_actions(request.excluded_actions)?,
            )?;
        }
        self.input.flush()?;
        let header = read_protocol_line(&mut self.output)?;
        if header != format!("BATCH {}", requests.len()) {
            return Err(format!("malformed policy batch header `{header}`").into());
        }
        let mut decisions = Vec::with_capacity(requests.len());
        for _ in requests {
            let response = read_protocol_line(&mut self.output)?;
            if let Some(error) = response.strip_prefix("ERROR ") {
                return Err(format!("policy process: {error}").into());
            }
            decisions.push(parse_oracle_decision(&response)?);
        }
        Ok(decisions)
    }
}

fn encode_excluded_actions(actions: &[SemanticAction]) -> Result<String> {
    if actions.is_empty() {
        return Ok("-".into());
    }
    actions
        .iter()
        .map(|action| Ok(action.encode_u63()?.to_string()))
        .collect::<Result<Vec<_>>>()
        .map(|encoded| encoded.join(","))
}

fn parse_oracle_decision(response: &str) -> Result<OracleDecision> {
    let fields: Vec<_> = response.split_whitespace().collect();
    match fields.as_slice() {
        ["APPLY", encoded, controller] => Ok(OracleDecision::Apply {
            action: parse_action(encoded)?,
            controller_plies: parse_u32(controller, "controller ply count")?,
        }),
        ["STOP_CC", encoded, controller] => Ok(OracleDecision::PreferredCrossingChange {
            action: parse_action(encoded)?,
            controller_plies: parse_u32(controller, "controller ply count")?,
        }),
        ["STOP_PASS", controller] => Ok(OracleDecision::PreferredPass {
            controller_plies: parse_u32(controller, "controller ply count")?,
        }),
        ["STOP_PLY_LIMIT", controller] => Ok(OracleDecision::PolicyPlyLimit {
            controller_plies: parse_u32(controller, "controller ply count")?,
        }),
        ["STOP_CONTROLLER_CYCLE", controller] => Ok(OracleDecision::ControllerCycle {
            controller_plies: parse_u32(controller, "controller ply count")?,
        }),
        ["STOP_TERMINATED", controller] => Ok(OracleDecision::EnvironmentTerminated {
            controller_plies: parse_u32(controller, "controller ply count")?,
        }),
        _ => Err(format!("malformed policy-process response `{response}`").into()),
    }
}

impl Drop for ExternalPolicyOracle {
    fn drop(&mut self) {
        let _ = writeln!(self.input, "QUIT");
        let _ = self.input.flush();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn read_protocol_line(reader: &mut BufReader<ChildStdout>) -> Result<String> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err("policy process closed its output".into());
    }
    if line.len() > MAX_PROTOCOL_LINE {
        return Err("policy-process response exceeds the line limit".into());
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn parse_action(value: &str) -> Result<SemanticAction> {
    let encoded = value
        .parse::<u64>()
        .map_err(|_| format!("bad encoded semantic action `{value}`"))?;
    SemanticAction::decode_u63(encoded)
}

fn parse_u32(value: &str, label: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .map_err(|_| format!("bad {label} `{value}`").into())
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FakeOracle {
        decisions: VecDeque<OracleDecision>,
    }

    impl FakeOracle {
        fn new(decisions: impl IntoIterator<Item = OracleDecision>) -> Self {
            Self {
                decisions: decisions.into_iter().collect(),
            }
        }
    }

    impl PolicyOracle for FakeOracle {
        fn model_id(&self) -> &str {
            "fake:Q254:pinned-sha256"
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
            self.decisions
                .pop_front()
                .ok_or("fake oracle exhausted".into())
        }

        fn first_semantic_decision_excluding(
            &mut self,
            normalized_source: &BraidRepresentation,
            remaining_policy_plies: u32,
            _excluded_actions: &[SemanticAction],
        ) -> Result<OracleDecision> {
            self.first_semantic_decision(normalized_source, remaining_policy_plies)
        }
    }

    fn braid(strands: u16, word: &[i16]) -> BraidRepresentation {
        BraidRepresentation {
            strands,
            cyclic_band_generators: false,
            word: word.to_vec(),
        }
    }

    #[test]
    fn policy_expansion_is_normalized_but_not_reduced_before_the_next_decision() {
        let mut oracle = FakeOracle::new([
            OracleDecision::Apply {
                action: SemanticAction::Commute { position: 0 },
                controller_plies: 2,
            },
            OracleDecision::PreferredCrossingChange {
                action: SemanticAction::CrossingChange { position: 0 },
                controller_plies: 1,
            },
        ]);
        let report = preprocess_to_stopping_point(
            &braid(5, &[-1, 3, 1, 2]),
            &mut oracle,
            PolicyLimits::default(),
        )
        .unwrap();
        assert_eq!(report.rounds.len(), 2);
        assert!(report.initial_reduction.program.actions.is_empty());
        assert_eq!(
            report.rounds[0].source.representation,
            braid(5, &[-1, 3, 1, 2])
        );
        assert_eq!(
            report.rounds[1].source.representation,
            braid(5, &[-1, 1, 2, 3])
        );
        assert_eq!(report.output.representation, braid(5, &[-1, 1, 2, 3]));
        assert!(SemanticAction::Reduce { position: 0 }
            .apply(&report.output.representation)
            .is_ok());
        assert_eq!(
            report.stop_reason,
            PolicyStopReason::PreferredCrossingChange
        );
        assert!(report.is_graph_stopping_point());
        assert_eq!(report.policy_plies, 4);
        report.verify().unwrap();
    }

    #[test]
    fn one_cc_edge_compiles_intermediate_normalization_into_replay_program() {
        let mut oracle = FakeOracle::new([
            OracleDecision::PreferredCrossingChange {
                action: SemanticAction::CrossingChange { position: 1 },
                controller_plies: 0,
            },
            OracleDecision::PreferredCrossingChange {
                action: SemanticAction::CrossingChange { position: 0 },
                controller_plies: 1,
            },
        ]);
        let source = braid(3, &[-1, 2, 2]);
        assert!(source.is_normalized().unwrap());
        let edge = trace_policy_edge(&source, &mut oracle, PolicyLimits::default()).unwrap();
        assert_eq!(edge.target.representation, braid(3, &[1]));
        assert_eq!(
            edge.program.instructions,
            vec![
                ProofInstruction::Action(SemanticAction::CrossingChange { position: 1 }),
                ProofInstruction::MirrorOrbit,
                ProofInstruction::NormalizeOrigin {
                    rotate_word_left: 2
                },
                ProofInstruction::Action(SemanticAction::Reduce { position: 2 }),
            ]
        );
        assert_eq!(edge.program.cc_cost(), 1);
        let concrete = edge
            .program
            .materialize_for_chirality(&edge.source.representation, false)
            .unwrap();
        assert!(concrete.target_mirrored);
        assert_eq!(concrete.target, braid(3, &[-1]));
        assert!(!concrete
            .program
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, ProofInstruction::MirrorOrbit)));
        edge.verify().unwrap();
    }

    #[test]
    fn representation_cycle_proposal_is_not_admitted() {
        let first = OracleDecision::Apply {
            action: SemanticAction::Commute { position: 0 },
            controller_plies: 0,
        };
        let cycle = OracleDecision::Apply {
            action: SemanticAction::Commute { position: 2 },
            controller_plies: 0,
        };
        let mut oracle = FakeOracle::new(std::iter::once(first).chain(std::iter::repeat_n(
            cycle,
            MAX_REPRESENTATION_CYCLE_SKIPS + 1,
        )));
        let report = preprocess_to_stopping_point(
            &braid(6, &[1, 3, 2]),
            &mut oracle,
            PolicyLimits::default(),
        )
        .unwrap();
        assert_eq!(report.rounds.len(), 2);
        assert!(matches!(
            report.rounds[1].decision,
            PolicyRoundDecision::RepresentationCycle { .. }
        ));
        assert_eq!(report.semantic_moves, 1);
        assert!(!report.is_graph_stopping_point());
        report.verify().unwrap();
    }

    #[test]
    fn quotient_self_loop_is_skipped_for_the_next_policy_preference() {
        struct SelfLoopOracle {
            calls: usize,
        }
        impl PolicyOracle for SelfLoopOracle {
            fn model_id(&self) -> &str {
                "fake:Q254:pinned-sha256"
            }
            fn objective_ratio(&self) -> u32 {
                GRAPH_STOP_OBJECTIVE_RATIO
            }
            fn controller_initial_state(&self) -> &str {
                CONTROLLER_INITIAL_STATE
            }
            fn first_semantic_decision(
                &mut self,
                source: &BraidRepresentation,
                remaining: u32,
            ) -> Result<OracleDecision> {
                self.first_semantic_decision_excluding(source, remaining, &[])
            }
            fn first_semantic_decision_excluding(
                &mut self,
                _source: &BraidRepresentation,
                _remaining: u32,
                excluded: &[SemanticAction],
            ) -> Result<OracleDecision> {
                let decision = match self.calls {
                    0 => OracleDecision::Apply {
                        action: SemanticAction::Insert {
                            position: 8,
                            generator: 3,
                            sign: 1,
                        },
                        controller_plies: 0,
                    },
                    1 => OracleDecision::Apply {
                        action: SemanticAction::Reduce { position: 10 },
                        controller_plies: 0,
                    },
                    2 => {
                        assert_eq!(excluded, &[SemanticAction::Reduce { position: 10 }]);
                        OracleDecision::PreferredCrossingChange {
                            action: SemanticAction::CrossingChange { position: 0 },
                            controller_plies: 0,
                        }
                    }
                    _ => return Err("unexpected extra oracle call".into()),
                };
                self.calls += 1;
                Ok(decision)
            }
        }

        let input = braid(4, &[-2, -1, -1, -2, 1, 3, 2, 3, 3]);
        let mut oracle = SelfLoopOracle { calls: 0 };
        let report =
            preprocess_to_stopping_point(&input, &mut oracle, PolicyLimits::default()).unwrap();
        assert_eq!(oracle.calls, 3);
        assert_eq!(
            report.stop_reason,
            PolicyStopReason::PreferredCrossingChange
        );
        assert_eq!(report.semantic_moves, 1);
        assert_eq!(report.rounds.len(), 2);
        assert!(report.is_graph_stopping_point());
        report.verify().unwrap();
    }

    #[test]
    fn oracle_cannot_smuggle_a_crossing_change_into_apply() {
        let mut oracle = FakeOracle::new([OracleDecision::Apply {
            action: SemanticAction::CrossingChange { position: 0 },
            controller_plies: 0,
        }]);
        assert!(preprocess_to_stopping_point(
            &braid(3, &[1, 2, 1, 2]),
            &mut oracle,
            PolicyLimits::default()
        )
        .is_err());
    }

    #[test]
    fn terminal_unknot_needs_no_policy_decision() {
        let mut oracle = FakeOracle::new([]);
        let report =
            preprocess_to_stopping_point(&braid(1, &[]), &mut oracle, PolicyLimits::default())
                .unwrap();
        assert_eq!(report.stop_reason, PolicyStopReason::TerminalRepresentation);
        assert!(report.is_graph_stopping_point());
        report.verify().unwrap();
    }

    #[test]
    fn batched_preprocessing_is_scalar_equivalent() {
        struct ImmediateCcOracle {
            batch_calls: usize,
        }
        impl PolicyOracle for ImmediateCcOracle {
            fn model_id(&self) -> &str {
                "fake:Q254:pinned-sha256"
            }
            fn objective_ratio(&self) -> u32 {
                GRAPH_STOP_OBJECTIVE_RATIO
            }
            fn controller_initial_state(&self) -> &str {
                CONTROLLER_INITIAL_STATE
            }
            fn first_semantic_decision(
                &mut self,
                source: &BraidRepresentation,
                _remaining: u32,
            ) -> Result<OracleDecision> {
                let position = source
                    .word
                    .iter()
                    .position(|letter| *letter != 0)
                    .ok_or("test source has no crossing")?;
                Ok(OracleDecision::PreferredCrossingChange {
                    action: SemanticAction::CrossingChange {
                        position: position.try_into()?,
                    },
                    controller_plies: 0,
                })
            }
            fn first_semantic_decisions_excluding(
                &mut self,
                requests: &[PolicyDecisionRequest<'_>],
            ) -> Result<Vec<OracleDecision>> {
                self.batch_calls += 1;
                requests
                    .iter()
                    .map(|request| {
                        self.first_semantic_decision(request.source, request.remaining_policy_plies)
                    })
                    .collect()
            }
        }

        let inputs = [braid(2, &[1, 1, 1]), braid(3, &[1, 2, 1, 2])];
        let limits = PolicyLimits::default();
        let mut scalar = ImmediateCcOracle { batch_calls: 0 };
        let expected: Vec<_> = inputs
            .iter()
            .map(|input| preprocess_to_stopping_point(input, &mut scalar, limits).unwrap())
            .collect();
        let mut batch = ImmediateCcOracle { batch_calls: 0 };
        let actual = preprocess_many_to_stopping_points(&inputs, &mut batch, limits).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(batch.batch_calls, 1);
    }
}
