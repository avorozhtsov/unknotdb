//! Transactional, bounded insertion of one query representation.
//!
//! Nothing is added unless a replay-checked track reaches a node that already
//! has a finite route to the unknot. Policy output only orders proposals; the
//! exact representation and proof-program validators remain authoritative.

use crate::policy::{
    compile_preprocessing_instructions, preprocess_to_stopping_point, preprocessing_audit_sha256,
    PolicyLimits, PolicyOracle, PolicyStopReason,
};
use crate::population::{PopulationGraph, RelaxationOutcome};
use crate::representation::{
    BraidRepresentation, CheckpointedProofProgram, NormalizedRepresentation, ProofInstruction,
    SemanticAction,
};
use crate::{PolicyStopAttestation, RepKey, Result};
use std::collections::{HashSet, VecDeque};

pub const CONNECTED_INSERT_MANIFEST_VERSION: &str = "unknotdb-connected-insert-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectedInsertLimits {
    pub max_simulations: u32,
    pub max_search_states: u32,
    pub max_track_depth: u16,
    pub policy_limits: PolicyLimits,
}

impl Default for ConnectedInsertLimits {
    fn default() -> Self {
        Self {
            max_simulations: 250,
            max_search_states: 250,
            max_track_depth: 12,
            policy_limits: PolicyLimits::default(),
        }
    }
}

impl ConnectedInsertLimits {
    fn validate(self) -> Result<()> {
        if self.max_simulations == 0 || self.max_search_states == 0 || self.max_track_depth == 0 {
            return Err("connected-insert budgets must be positive".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectedInsertDisposition {
    AlreadyCovered { u_upper_bound: u32 },
    Inserted { u_upper_bound: u32 },
    CoverageMiss { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolveComparison {
    pub policy_only_found: bool,
    pub policy_only_u_upper_bound: Option<u32>,
    pub policy_only_simulations: u32,
    pub graph_assisted_found: bool,
    pub graph_assisted_u_upper_bound: Option<u32>,
    pub graph_assisted_simulations: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectedInsertResult {
    pub disposition: ConnectedInsertDisposition,
    pub input_key: RepKey,
    pub stopping_key: Option<RepKey>,
    pub simulations: u32,
    pub searched_states: u32,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub inverse_witnesses_replayed: usize,
    pub comparison: SolveComparison,
    pub manifest: String,
}

#[derive(Clone)]
struct SearchState {
    stopping: NormalizedRepresentation,
    attestation: PolicyStopAttestation,
    parent: Option<usize>,
    edge_from_parent: Option<CheckpointedProofProgram>,
    depth: u16,
    policy_chain: bool,
}

/// Add the preprocessed stopping point for `input`, but only if a bounded,
/// exact track reaches an already connected graph node. Mutation is
/// transactional: a coverage miss leaves `graph` byte-for-byte logically
/// unchanged.
pub fn force_connected_insert<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    input: &BraidRepresentation,
    oracle: &mut O,
    limits: ConnectedInsertLimits,
    source_id: &str,
    snapshot_id: &str,
) -> Result<ConnectedInsertResult> {
    limits.validate()?;
    input.validate()?;
    if source_id.is_empty() || snapshot_id.is_empty() {
        return Err("connected-insert source and snapshot IDs must be non-empty".into());
    }
    let input_key = input.normalize()?.key;
    let initial = preprocess_to_stopping_point(input, oracle, limits.policy_limits)?;
    let mut manifest = manifest_header(
        input,
        input_key,
        oracle,
        limits,
        source_id,
        snapshot_id,
        &initial,
    )?;
    if !initial.is_graph_stopping_point() {
        let reason = format!("initial preprocessing stopped at {:?}", initial.stop_reason);
        return Ok(miss_result(input_key, None, reason, manifest));
    }
    let start = initial.output.clone();
    if let Some(u) = graph.u_upper_bound(&start.key) {
        let terminal = initial.stop_reason == PolicyStopReason::TerminalRepresentation;
        manifest.push_str(&format!("result\talready_covered\t{}\n", u));
        return Ok(ConnectedInsertResult {
            disposition: ConnectedInsertDisposition::AlreadyCovered { u_upper_bound: u },
            input_key,
            stopping_key: Some(start.key),
            simulations: 0,
            searched_states: 1,
            inserted_nodes: 0,
            inserted_edges: 0,
            inverse_witnesses_replayed: 0,
            comparison: SolveComparison {
                policy_only_found: terminal,
                policy_only_u_upper_bound: terminal.then_some(0),
                policy_only_simulations: 0,
                graph_assisted_found: true,
                graph_assisted_u_upper_bound: Some(u),
                graph_assisted_simulations: 0,
            },
            manifest,
        });
    }

    let start_attestation = attestation(&initial)?;
    let initial_state = SearchState {
        stopping: start.clone(),
        attestation: start_attestation,
        parent: None,
        edge_from_parent: None,
        depth: 0,
        policy_chain: true,
    };
    let mut simulations = 0_u32;
    let mut policy_states = vec![initial_state.clone()];
    let mut policy_seen = HashSet::from([start.key]);
    let mut policy_terminal = None;
    while simulations < limits.max_simulations
        && policy_states.len() < limits.max_search_states as usize
    {
        let current_index = policy_states.len() - 1;
        let current = policy_states[current_index].clone();
        if current.depth >= limits.max_track_depth {
            break;
        }
        let Some(action) = preferred_cc(current.attestation)? else {
            break;
        };
        let raw = action.apply(&current.stopping.representation)?;
        simulations += 1;
        let report = preprocess_to_stopping_point(&raw, oracle, limits.policy_limits)?;
        if !report.is_graph_stopping_point() || !policy_seen.insert(report.output.key) {
            break;
        }
        let mut instructions = vec![ProofInstruction::Action(action)];
        instructions.extend(compile_preprocessing_instructions(&report)?);
        let program = CheckpointedProofProgram { instructions }
            .canonicalize_semantic_first(&current.stopping.representation)?;
        let next_index = policy_states.len();
        policy_states.push(SearchState {
            stopping: report.output.clone(),
            attestation: attestation(&report)?,
            parent: Some(current_index),
            edge_from_parent: Some(program),
            depth: current.depth + 1,
            policy_chain: true,
        });
        if let Some(target_u) = graph.u_upper_bound(&report.output.key) {
            policy_terminal = Some((next_index, report.output.key, target_u));
            break;
        }
    }
    let policy_only_simulations = simulations;
    let policy_only_u = policy_terminal
        .map(|(index, _, target_u)| target_u + u32::from(policy_states[index].depth));

    let mut states = vec![initial_state];
    let mut queue = VecDeque::from([0_usize]);
    let mut seen = HashSet::from([start.key]);
    let mut terminal: Option<(usize, RepKey, u32)> = None;

    // Breadth-first graph-aware search. At every state the frozen policy's
    // preferred CC is tried first; remaining legal CCs provide bounded
    // robustness. Each successor is independently preprocessed and replayed.
    while let Some(state_index) = queue.pop_front() {
        if simulations >= limits.max_simulations
            || states.len() + policy_states.len() > limits.max_search_states as usize
        {
            break;
        }
        let state = states[state_index].clone();
        if state.depth >= limits.max_track_depth {
            continue;
        }
        let preferred = preferred_cc(state.attestation)?;
        let mut actions = Vec::new();
        if let Some(action) = preferred {
            actions.push(action);
        }
        actions.extend(
            (0..state.stopping.representation.word.len() as u32)
                .map(|position| SemanticAction::CrossingChange { position })
                .filter(|action| Some(*action) != preferred),
        );
        for action in actions {
            if simulations >= limits.max_simulations
                || states.len() + policy_states.len() > limits.max_search_states as usize
            {
                break;
            }
            let Ok(raw) = action.apply(&state.stopping.representation) else {
                continue;
            };
            simulations += 1;
            let report = preprocess_to_stopping_point(&raw, oracle, limits.policy_limits)?;
            if !report.is_graph_stopping_point() {
                manifest.push_str(&format!(
                    "attempt\t{}\t{}\tincomplete:{:?}\n",
                    state.depth + 1,
                    action.encode_u63()?,
                    report.stop_reason
                ));
                continue;
            }
            let mut instructions = vec![ProofInstruction::Action(action)];
            instructions.extend(compile_preprocessing_instructions(&report)?);
            let program = CheckpointedProofProgram { instructions }
                .canonicalize_semantic_first(&state.stopping.representation)?;
            if program.replay(&state.stopping.representation)? != report.output.representation {
                return Err("connected-insert successor failed exact replay".into());
            }
            let inverse = program.invert_edge(&state.stopping.representation)?;
            if inverse.program.replay(&inverse.source)? != inverse.target {
                return Err("connected-insert inverse witness failed exact replay".into());
            }
            let next_key = report.output.key;
            let is_policy_child = preferred == Some(action);
            let next_policy_chain = state.policy_chain && is_policy_child;
            if let Some(target_u) = graph.u_upper_bound(&next_key) {
                let next_index = states.len();
                states.push(SearchState {
                    stopping: report.output.clone(),
                    attestation: attestation(&report)?,
                    parent: Some(state_index),
                    edge_from_parent: Some(program),
                    depth: state.depth + 1,
                    policy_chain: next_policy_chain,
                });
                terminal = Some((next_index, next_key, target_u));
                manifest.push_str(&format!(
                    "hit\t{}\t{}\t{}\t{}\n",
                    state.depth + 1,
                    hex(&next_key),
                    target_u,
                    action.encode_u63()?
                ));
                break;
            }
            if seen.insert(next_key) {
                let next_attestation = attestation(&report)?;
                let next_index = states.len();
                states.push(SearchState {
                    stopping: report.output,
                    attestation: next_attestation,
                    parent: Some(state_index),
                    edge_from_parent: Some(program),
                    depth: state.depth + 1,
                    policy_chain: next_policy_chain,
                });
                queue.push_back(next_index);
            }
        }
        if terminal.is_some() {
            break;
        }
    }

    let graph_simulations = simulations - policy_only_simulations;
    let searched_states = (states.len() + policy_states.len() - 1) as u32;
    let graph_u = terminal.map(|(index, _, u)| u + u32::from(states[index].depth));
    let comparison = SolveComparison {
        policy_only_found: policy_only_u.is_some(),
        policy_only_u_upper_bound: policy_only_u,
        policy_only_simulations,
        graph_assisted_found: terminal.is_some(),
        graph_assisted_u_upper_bound: graph_u,
        graph_assisted_simulations: graph_simulations,
    };
    manifest.push_str(&format!(
        "comparison\t{}\t{}\t{}\t{}\t{}\t{}\n",
        u8::from(comparison.policy_only_found),
        optional_u32(comparison.policy_only_u_upper_bound),
        comparison.policy_only_simulations,
        u8::from(comparison.graph_assisted_found),
        optional_u32(comparison.graph_assisted_u_upper_bound),
        comparison.graph_assisted_simulations
    ));
    let use_policy = policy_only_u.is_some() && (graph_u.is_none() || policy_only_u <= graph_u);
    if use_policy {
        states = policy_states;
        terminal = policy_terminal;
    }
    let Some((mut cursor, _, _)) = terminal else {
        let reason = format!(
            "budget exhausted without a finite-U graph hit (simulations={}, states={})",
            simulations,
            states.len()
        );
        manifest.push_str(&format!("result\tcoverage_miss\t{}\n", reason));
        let mut result = miss_result(input_key, Some(start.key), reason, manifest);
        result.simulations = simulations;
        result.searched_states = searched_states;
        result.comparison = comparison;
        return Ok(result);
    };

    let old_nodes = graph.node_count();
    let old_edges = graph.edge_count();
    let mut staged = graph.clone();
    let mut inverse_witnesses_replayed = 0_usize;
    while let Some(parent) = states[cursor].parent {
        let program = states[cursor]
            .edge_from_parent
            .clone()
            .ok_or("connected track is missing its edge")?;
        let inverse = program.invert_edge(&states[parent].stopping.representation)?;
        inverse_witnesses_replayed += 1;
        manifest.push_str(&format!(
            "inverse\t{}\treplayed_not_admitted_unless_route_useful\t{}\n",
            hex(&unknotdb::util::sha256(&inverse.program.encode()?)),
            inverse.program.cc_cost()
        ));
        let program_sha256 = unknotdb::util::sha256(&program.encode()?);
        let outcome = staged.relax_unknot_edge_deferred_acs10(
            states[parent].stopping.clone(),
            states[parent].attestation,
            states[cursor].stopping.key,
            program,
            None,
        )?;
        manifest.push_str(&format!(
            "edge\t{}\t{}\t{}\t{:?}\n",
            hex(&states[parent].stopping.key),
            hex(&states[cursor].stopping.key),
            hex(&program_sha256),
            outcome
        ));
        if !matches!(
            outcome,
            RelaxationOutcome::Inserted { .. } | RelaxationOutcome::Improved { .. }
        ) {
            return Err("connected track did not improve or insert its source".into());
        }
        cursor = parent;
    }
    staged.recompute_acs10_routes()?;
    let resulting_u = staged
        .u_upper_bound(&start.key)
        .ok_or("connected insertion did not retain a finite U route")?;
    let inserted_nodes = staged.node_count() - old_nodes;
    let inserted_edges = staged.edge_count() - old_edges;
    manifest.push_str(&format!(
        "result\tinserted\t{}\t{}\t{}\t{}\n",
        resulting_u, inserted_nodes, inserted_edges, inverse_witnesses_replayed
    ));
    *graph = staged;
    Ok(ConnectedInsertResult {
        disposition: ConnectedInsertDisposition::Inserted {
            u_upper_bound: resulting_u,
        },
        input_key,
        stopping_key: Some(start.key),
        simulations,
        searched_states,
        inserted_nodes,
        inserted_edges,
        inverse_witnesses_replayed,
        comparison,
        manifest,
    })
}

fn attestation(report: &crate::policy::PreprocessingReport) -> Result<PolicyStopAttestation> {
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
        PolicyStopReason::CapacityFallback => {
            Ok(PolicyStopAttestation::CapacityFallback { audit_sha256 })
        }
        _ => Err("incomplete preprocessing has no graph attestation".into()),
    }
}

fn preferred_cc(attestation: PolicyStopAttestation) -> Result<Option<SemanticAction>> {
    match attestation {
        PolicyStopAttestation::PreferredCrossingChange { action, .. } => {
            Ok(Some(SemanticAction::decode_u63(action)?))
        }
        PolicyStopAttestation::Terminal { .. } => Ok(None),
        PolicyStopAttestation::CapacityFallback { .. } => Ok(None),
    }
}

fn manifest_header<O: PolicyOracle>(
    input: &BraidRepresentation,
    input_key: RepKey,
    oracle: &O,
    limits: ConnectedInsertLimits,
    source_id: &str,
    snapshot_id: &str,
    initial: &crate::policy::PreprocessingReport,
) -> Result<String> {
    Ok(format!(
        "{CONNECTED_INSERT_MANIFEST_VERSION}\nsource_id\t{source_id}\nsnapshot_id\t{snapshot_id}\nmodel_id\t{}\nobjective_ratio\t{}\ninput\t{}\t{}\nlimits\t{}\t{}\t{}\t{}\t{}\ninitial\t{:?}\t{}\t{}\n",
        oracle.model_id(), oracle.objective_ratio(), hex(&input_key), hex(&input.encode()?),
        limits.max_simulations, limits.max_search_states, limits.max_track_depth,
        limits.policy_limits.max_policy_plies, limits.policy_limits.max_semantic_moves,
        initial.stop_reason, initial.policy_plies, initial.semantic_moves
    ))
}

fn miss_result(
    input_key: RepKey,
    stopping_key: Option<RepKey>,
    reason: String,
    manifest: String,
) -> ConnectedInsertResult {
    ConnectedInsertResult {
        disposition: ConnectedInsertDisposition::CoverageMiss { reason },
        input_key,
        stopping_key,
        simulations: 0,
        searched_states: 1,
        inserted_nodes: 0,
        inserted_edges: 0,
        inverse_witnesses_replayed: 0,
        comparison: SolveComparison {
            policy_only_found: false,
            policy_only_u_upper_bound: None,
            policy_only_simulations: 0,
            graph_assisted_found: false,
            graph_assisted_u_upper_bound: None,
            graph_assisted_simulations: 0,
        },
        manifest,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn optional_u32(value: Option<u32>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "-".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{OracleDecision, CONTROLLER_INITIAL_STATE, GRAPH_STOP_OBJECTIVE_RATIO};

    struct PreferredCcOracle;
    impl PolicyOracle for PreferredCcOracle {
        fn model_id(&self) -> &str {
            "fake:Q254:connected-insert-test"
        }
        fn objective_ratio(&self) -> u32 {
            GRAPH_STOP_OBJECTIVE_RATIO
        }
        fn controller_initial_state(&self) -> &str {
            CONTROLLER_INITIAL_STATE
        }
        fn first_semantic_decision(
            &mut self,
            _source: &BraidRepresentation,
            _: u32,
        ) -> Result<OracleDecision> {
            Ok(OracleDecision::PreferredCrossingChange {
                action: SemanticAction::CrossingChange { position: 0 },
                controller_plies: 1,
            })
        }
    }

    struct PassOracle;
    impl PolicyOracle for PassOracle {
        fn model_id(&self) -> &str {
            "fake:Q254:connected-insert-test"
        }
        fn objective_ratio(&self) -> u32 {
            GRAPH_STOP_OBJECTIVE_RATIO
        }
        fn controller_initial_state(&self) -> &str {
            CONTROLLER_INITIAL_STATE
        }
        fn first_semantic_decision(
            &mut self,
            _: &BraidRepresentation,
            _: u32,
        ) -> Result<OracleDecision> {
            Ok(OracleDecision::PreferredPass {
                controller_plies: 1,
            })
        }
    }

    #[test]
    fn inserts_trefoil_only_after_reaching_known_unknot() {
        let mut graph = PopulationGraph::from_unknot([7; 32]).unwrap();
        let trefoil = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        };
        let before = (graph.node_count(), graph.edge_count());
        let result = force_connected_insert(
            &mut graph,
            &trefoil,
            &mut PreferredCcOracle,
            ConnectedInsertLimits::default(),
            "test",
            "root",
        )
        .unwrap();
        assert!(matches!(
            result.disposition,
            ConnectedInsertDisposition::Inserted { u_upper_bound: 1 }
        ));
        assert_eq!(graph.node_count(), before.0 + 1);
        assert_eq!(graph.edge_count(), before.1 + 1);
        assert_eq!(result.inverse_witnesses_replayed, 1);
        assert!(result.manifest.contains("result\tinserted\t1"));
    }

    #[test]
    fn incomplete_preprocessing_is_a_transactional_miss() {
        let mut graph = PopulationGraph::from_unknot([7; 32]).unwrap();
        let trefoil = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        };
        let before = (graph.node_count(), graph.edge_count());
        let result = force_connected_insert(
            &mut graph,
            &trefoil,
            &mut PassOracle,
            ConnectedInsertLimits::default(),
            "test",
            "root",
        )
        .unwrap();
        assert!(matches!(
            result.disposition,
            ConnectedInsertDisposition::CoverageMiss { .. }
        ));
        assert_eq!((graph.node_count(), graph.edge_count()), before);
    }

    #[test]
    fn exhausted_budget_never_persists_a_disconnected_stopping_point() {
        let mut graph = PopulationGraph::from_unknot([7; 32]).unwrap();
        let cinquefoil = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1, 1, 1],
        };
        let before = (graph.node_count(), graph.edge_count());
        let result = force_connected_insert(
            &mut graph,
            &cinquefoil,
            &mut PreferredCcOracle,
            ConnectedInsertLimits {
                max_simulations: 1,
                ..ConnectedInsertLimits::default()
            },
            "test",
            "root",
        )
        .unwrap();
        assert!(matches!(
            result.disposition,
            ConnectedInsertDisposition::CoverageMiss { .. }
        ));
        assert_eq!((graph.node_count(), graph.edge_count()), before);
        assert_eq!(result.simulations, 1);
    }
}
