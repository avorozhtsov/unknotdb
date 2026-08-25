//! Reproducible, graph-aware optimization of one existing stopping point.
//!
//! Search proposals are untrusted. A run is committed to the mutable graph
//! only when an independently replayed immutable edge strictly lowers the
//! selected node's verified U upper bound. The final Bellman refresh retains
//! any indirect improvements implied by all stored proof edges.

use crate::frontier::{enumerate_scrambles_detailed, ScrambleLimits};
use crate::population::{PopulationGraph, RelaxationOutcome};
use crate::representation::{CheckpointedProofProgram, NormalizationWitness, ProofInstruction};
use crate::{RepKey, Result};
use std::collections::{BTreeMap, BTreeSet};

pub const OPTIMIZER_MANIFEST_VERSION: &str = "unknotdb-vertex-optimizer-v0";

/// Deterministic built-in proposal ordering. These profiles do not assert
/// knot facts and do not replace the snapshot-pinned neural policy contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptimizerPolicyProfile {
    /// Prefer the smallest graph-backed resulting U, then the shortest proof.
    GraphAware,
    /// Prefer the first shallow exact hit, with graph U as a tie breaker.
    BreadthFirst,
    /// Prefer the shortest semantic witness, with graph U as a tie breaker.
    ShortWitness,
}

impl OptimizerPolicyProfile {
    fn name(self) -> &'static str {
        match self {
            Self::GraphAware => "graph-aware",
            Self::BreadthFirst => "breadth-first",
            Self::ShortWitness => "short-witness",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptimizerBudget {
    pub max_depth: u16,
    /// Aggregate visited-state charge across all attempted depths.
    pub max_states: u32,
    /// Aggregate exact candidate evaluations ("simulations").
    pub max_simulations: u32,
    pub max_strands: u16,
    pub max_word_length: u32,
}

impl OptimizerBudget {
    pub fn validate(self) -> Result<()> {
        if self.max_depth == 0
            || self.max_states == 0
            || self.max_simulations == 0
            || self.max_strands == 0
        {
            return Err("optimizer depth/state/simulation/strand budgets must be positive".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizerProvenance {
    pub snapshot_id: String,
    pub policy_model_id: String,
}

impl OptimizerProvenance {
    fn validate(&self) -> Result<()> {
        if self.snapshot_id.trim().is_empty() || self.policy_model_id.trim().is_empty() {
            return Err("optimizer snapshot and policy model IDs must be non-empty".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizerDepthAudit {
    pub depth: u16,
    pub visited_states: u32,
    pub candidates: u32,
    pub state_truncated: bool,
    pub candidate_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizerRun {
    pub provenance: OptimizerProvenance,
    pub profile: OptimizerPolicyProfile,
    pub budget: OptimizerBudget,
    pub source_key: RepKey,
    pub old_u: u32,
    pub new_u: u32,
    pub success: bool,
    pub visited_states: u32,
    pub simulations: u32,
    pub duplicate_or_cycle_exclusions: u32,
    pub exact_graph_hits: u32,
    pub accepted_edges: u32,
    pub indirect_improvements: u32,
    pub depth_audit: Vec<OptimizerDepthAudit>,
    pub accepted_target: Option<RepKey>,
    pub accepted_program: Option<Vec<u8>>,
}

impl OptimizerRun {
    pub fn manifest_tsv(&self) -> String {
        let mut text = format!(
            "{OPTIMIZER_MANIFEST_VERSION}\n\
snapshot_id\t{}\npolicy_model_id\t{}\nprofile\t{}\n\
source_key\t{}\nbudget\t{}\t{}\t{}\t{}\t{}\n\
result\t{}\t{}\t{}\naccounting\t{}\t{}\t{}\t{}\t{}\n",
            self.provenance.snapshot_id,
            self.provenance.policy_model_id,
            self.profile.name(),
            hex(&self.source_key),
            self.budget.max_depth,
            self.budget.max_states,
            self.budget.max_simulations,
            self.budget.max_strands,
            self.budget.max_word_length,
            if self.success {
                "improved"
            } else {
                "no-improvement"
            },
            self.old_u,
            self.new_u,
            self.visited_states,
            self.simulations,
            self.duplicate_or_cycle_exclusions,
            self.exact_graph_hits,
            self.indirect_improvements,
        );
        text.push_str("depth\tdepth\tvisited\tcandidates\tstate_truncated\tcandidate_truncated\n");
        for audit in &self.depth_audit {
            text.push_str(&format!(
                "depth\t{}\t{}\t{}\t{}\t{}\n",
                audit.depth,
                audit.visited_states,
                audit.candidates,
                u8::from(audit.state_truncated),
                u8::from(audit.candidate_truncated),
            ));
        }
        text.push_str(&format!(
            "accepted\t{}\t{}\t{}\n",
            self.accepted_edges,
            self.accepted_target
                .as_ref()
                .map(|key| hex(key))
                .unwrap_or_else(|| "-".into()),
            self.accepted_program
                .as_deref()
                .map(hex)
                .unwrap_or_else(|| "-".into()),
        ));
        text
    }
}

#[derive(Clone)]
struct Proposal {
    depth: u16,
    target_key: RepKey,
    target_u: u32,
    program: CheckpointedProofProgram,
    encoded_program: Vec<u8>,
    semantic_len: u32,
}

/// Optimize one canonical node under a hard deterministic budget.
///
/// The graph is unchanged on `no-improvement` and on every error. Candidate
/// targets must already have finite verified routes in `graph`, so a successful
/// edge is graph-connected by construction.
pub fn optimize_vertex(
    graph: &mut PopulationGraph,
    source_key: RepKey,
    provenance: OptimizerProvenance,
    budget: OptimizerBudget,
    profile: OptimizerPolicyProfile,
) -> Result<OptimizerRun> {
    budget.validate()?;
    provenance.validate()?;
    let source = graph
        .expansion_seed(&source_key)?
        .ok_or("optimizer source key is absent from the graph")?;
    if source.representation.key != source_key
        || !source.representation.representation.is_normalized()?
    {
        return Err("optimizer source is not canonical".into());
    }
    if source.representation.representation.strands > budget.max_strands
        || source.representation.representation.word.len() > budget.max_word_length as usize
    {
        return Err("optimizer source already exceeds its geometry budget".into());
    }

    let old_u = source.u_upper_bound;
    let mut visited_states = 0_u32;
    let mut simulations = 0_u32;
    let mut excluded = 0_u32;
    let mut exact_graph_hits = 0_u32;
    let mut seen_targets = BTreeSet::from([source_key]);
    let mut proposals = BTreeMap::<RepKey, Proposal>::new();
    let mut depth_audit = Vec::new();

    for depth in 1..=budget.max_depth {
        let remaining_states = budget.max_states.saturating_sub(visited_states);
        let remaining_simulations = budget.max_simulations.saturating_sub(simulations);
        if remaining_states == 0 || remaining_simulations == 0 {
            break;
        }
        let enumeration = enumerate_scrambles_detailed(
            &source.representation.representation,
            ScrambleLimits {
                depth,
                max_states: remaining_states,
                max_candidates: remaining_simulations,
                max_strands: budget.max_strands,
                max_word_length: budget.max_word_length,
                min_cc_cost: 1,
                max_cc_cost: 1,
            },
        )?;
        visited_states = visited_states
            .checked_add(enumeration.visited_states)
            .ok_or("optimizer visited-state accounting overflow")?;
        let candidate_count: u32 = enumeration
            .candidates
            .len()
            .try_into()
            .map_err(|_| "optimizer candidate count exceeds u32")?;
        depth_audit.push(OptimizerDepthAudit {
            depth,
            visited_states: enumeration.visited_states,
            candidates: candidate_count,
            state_truncated: enumeration.truncated_by_states,
            candidate_truncated: enumeration.truncated_by_candidates,
        });
        for candidate in enumeration.candidates {
            if simulations >= budget.max_simulations {
                break;
            }
            simulations += 1;
            let normalized = candidate.output.normalize()?;
            if !seen_targets.insert(normalized.key) {
                excluded += 1;
                continue;
            }
            let Some(target_u) = graph.u_upper_bound(&normalized.key) else {
                continue;
            };
            exact_graph_hits += 1;
            if target_u.saturating_add(u32::from(candidate.cc_cost)) >= old_u {
                continue;
            }
            let mut instructions: Vec<_> = candidate
                .actions
                .iter()
                .copied()
                .map(ProofInstruction::Action)
                .collect();
            push_normalization(&mut instructions, normalized.witness);
            let program = CheckpointedProofProgram { instructions }
                .canonicalize_semantic_first(&source.representation.representation)?;
            if program.replay(&source.representation.representation)? != normalized.representation {
                return Err("optimizer proposal failed exact normalized replay".into());
            }
            if program.cc_cost() != 1 {
                return Err("optimizer proposal does not contain exactly one CC".into());
            }
            let encoded_program = program.encode()?;
            let semantic_len = program
                .instructions
                .iter()
                .filter(|instruction| matches!(instruction, ProofInstruction::Action(_)))
                .count()
                .try_into()
                .map_err(|_| "optimizer semantic length exceeds u32")?;
            let proposal = Proposal {
                depth,
                target_key: normalized.key,
                target_u,
                program,
                encoded_program,
                semantic_len,
            };
            proposals
                .entry(normalized.key)
                .and_modify(|current| {
                    if proposal_order(&proposal, profile) < proposal_order(current, profile) {
                        *current = proposal.clone();
                    }
                })
                .or_insert(proposal);
        }
    }

    let mut ordered: Vec<_> = proposals.into_values().collect();
    ordered.sort_by_key(|proposal| proposal_order(proposal, profile));
    let before: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let mut scratch = graph.clone();
    let mut accepted = None;
    for proposal in ordered {
        let outcome = scratch.relax_unknot_edge_deferred_acs10(
            source.representation.clone(),
            source.policy_stop,
            proposal.target_key,
            proposal.program.clone(),
            None,
        )?;
        if matches!(outcome, RelaxationOutcome::Improved { .. }) {
            accepted = Some(proposal);
            break;
        }
    }

    let mut new_u = old_u;
    let mut indirect_improvements = 0_u32;
    if accepted.is_some() {
        scratch.recompute_unknot_routes()?;
        scratch.recompute_acs10_routes()?;
        new_u = scratch
            .u_upper_bound(&source_key)
            .ok_or("optimized source disappeared during Bellman refresh")?;
        if new_u >= old_u {
            return Err("accepted optimizer edge did not strictly improve selected U".into());
        }
        for (key, after) in scratch.u_bounds() {
            if key != source_key && before.get(&key).is_some_and(|old| after < *old) {
                indirect_improvements += 1;
            }
        }
        *graph = scratch;
    }

    Ok(OptimizerRun {
        provenance,
        profile,
        budget,
        source_key,
        old_u,
        new_u,
        success: accepted.is_some(),
        visited_states,
        simulations,
        duplicate_or_cycle_exclusions: excluded,
        exact_graph_hits,
        accepted_edges: u32::from(accepted.is_some()),
        indirect_improvements,
        depth_audit,
        accepted_target: accepted.as_ref().map(|proposal| proposal.target_key),
        accepted_program: accepted.map(|proposal| proposal.encoded_program),
    })
}

fn proposal_order(
    proposal: &Proposal,
    profile: OptimizerPolicyProfile,
) -> (u32, u32, u32, u16, RepKey) {
    let proposed_u = proposal.target_u.saturating_add(1);
    match profile {
        OptimizerPolicyProfile::GraphAware => (
            proposed_u,
            proposal.semantic_len,
            proposal.target_u,
            proposal.depth,
            proposal.target_key,
        ),
        OptimizerPolicyProfile::BreadthFirst => (
            u32::from(proposal.depth),
            proposed_u,
            proposal.semantic_len,
            proposal.depth,
            proposal.target_key,
        ),
        OptimizerPolicyProfile::ShortWitness => (
            proposal.semantic_len,
            proposed_u,
            proposal.target_u,
            proposal.depth,
            proposal.target_key,
        ),
    }
}

fn push_normalization(instructions: &mut Vec<ProofInstruction>, witness: NormalizationWitness) {
    if witness.mirrored {
        instructions.push(ProofInstruction::MirrorOrbit);
    }
    if witness.rotate_word_left != 0 {
        instructions.push(ProofInstruction::NormalizeOrigin {
            rotate_word_left: witness.rotate_word_left,
        });
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::population::PopulationGraph;
    use crate::representation::{BraidRepresentation, SemanticAction};
    use crate::PolicyStopAttestation;

    fn stop(byte: u8) -> PolicyStopAttestation {
        PolicyStopAttestation::PreferredCrossingChange {
            action: SemanticAction::CrossingChange { position: 0 }
                .encode_u63()
                .unwrap(),
            audit_sha256: [byte; 32],
        }
    }

    #[test]
    fn invalid_budget_is_rejected_without_mutation() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let root = graph.unknot_key().unwrap();
        let before = graph.u_bounds();
        let error = optimize_vertex(
            &mut graph,
            root,
            OptimizerProvenance {
                snapshot_id: "test-snapshot".into(),
                policy_model_id: "frozen-test-policy".into(),
            },
            OptimizerBudget {
                max_depth: 0,
                max_states: 1,
                max_simulations: 1,
                max_strands: 1,
                max_word_length: 0,
            },
            OptimizerPolicyProfile::GraphAware,
        )
        .unwrap_err();
        assert!(error.to_string().contains("must be positive"));
        assert_eq!(graph.u_bounds(), before);
    }

    #[test]
    fn no_improvement_is_truthful_reproducible_and_non_mutating() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let root = graph.unknot_key().unwrap();
        let before_nodes = graph.node_count();
        let before_edges = graph.edge_count();
        let run = optimize_vertex(
            &mut graph,
            root,
            OptimizerProvenance {
                snapshot_id: "test-snapshot".into(),
                policy_model_id: "frozen-test-policy".into(),
            },
            OptimizerBudget {
                max_depth: 2,
                max_states: 64,
                max_simulations: 16,
                max_strands: 2,
                max_word_length: 4,
            },
            OptimizerPolicyProfile::GraphAware,
        )
        .unwrap();
        assert!(!run.success);
        assert_eq!(run.old_u, 0);
        assert_eq!(run.new_u, 0);
        assert_eq!(run.accepted_edges, 0);
        assert_eq!(graph.node_count(), before_nodes);
        assert_eq!(graph.edge_count(), before_edges);
        let first = run.manifest_tsv();
        let second = run.manifest_tsv();
        assert_eq!(first, second);
        assert!(first.contains("result\tno-improvement\t0\t0"));
    }

    #[test]
    fn exact_graph_hit_strictly_improves_and_is_replayable() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let root = graph.unknot_key().unwrap();
        let simple = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1],
        }
        .normalize()
        .unwrap();
        graph
            .relax_unknot_edge(
                simple.clone(),
                stop(2),
                root,
                CheckpointedProofProgram {
                    instructions: vec![
                        ProofInstruction::Action(SemanticAction::CrossingChange { position: 0 }),
                        ProofInstruction::Action(SemanticAction::Destabilize),
                    ],
                },
                None,
            )
            .unwrap();
        let source = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        }
        .normalize()
        .unwrap();
        graph
            .relax_unknot_edge(
                source.clone(),
                stop(3),
                simple.key,
                CheckpointedProofProgram {
                    instructions: vec![
                        ProofInstruction::Action(SemanticAction::CrossingChange { position: 0 }),
                        ProofInstruction::Action(SemanticAction::Reduce { position: 0 }),
                    ],
                },
                None,
            )
            .unwrap();
        assert_eq!(graph.u_upper_bound(&source.key), Some(2));

        let changed = SemanticAction::CrossingChange { position: 0 }
            .apply(&source.representation)
            .unwrap()
            .normalize()
            .unwrap();
        let mut zero_program = None;
        for position in 0..changed.representation.word.len() as u32 {
            let candidate = CheckpointedProofProgram {
                instructions: vec![
                    ProofInstruction::Action(SemanticAction::Reduce { position }),
                    ProofInstruction::Action(SemanticAction::Destabilize),
                ],
            };
            if candidate
                .replay(&changed.representation)
                .is_ok_and(|output| output.strands == 1 && output.word.is_empty())
            {
                zero_program = Some(candidate);
                break;
            }
        }
        graph
            .relax_unknot_edge(
                changed.clone(),
                stop(4),
                root,
                zero_program.expect("normalized CC output must reduce to the unknot"),
                None,
            )
            .unwrap();
        assert_eq!(graph.u_upper_bound(&changed.key), Some(0));
        let old_edges = graph.edge_count();

        let run = optimize_vertex(
            &mut graph,
            source.key,
            OptimizerProvenance {
                snapshot_id: "test-snapshot".into(),
                policy_model_id: "frozen-test-policy".into(),
            },
            OptimizerBudget {
                max_depth: 1,
                max_states: 32,
                max_simulations: 8,
                max_strands: 2,
                max_word_length: 3,
            },
            OptimizerPolicyProfile::GraphAware,
        )
        .unwrap();
        assert!(run.success);
        assert_eq!((run.old_u, run.new_u), (2, 1));
        assert_eq!(run.accepted_target, Some(changed.key));
        assert_eq!(run.accepted_edges, 1);
        assert_eq!(graph.edge_count(), old_edges + 1);
        assert_eq!(graph.u_upper_bound(&source.key), Some(1));
        assert!(run.manifest_tsv().contains("result\timproved\t2\t1"));
    }
}
