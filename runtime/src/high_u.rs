//! Bounded frozen-policy improvement of existing high-U graph vertices.

use crate::policy::{
    preprocess_to_stopping_point, preprocessing_audit_sha256,
    trace_crossing_change_edge_from_attestation, trace_policy_edge_from_attestation,
    PolicyEdgeTrace, PolicyLimits, PolicyOracle, PolicyStopReason, PreprocessingReport,
};
use crate::population::PopulationGraph;
use crate::representation::{CheckpointedProofProgram, NormalizedRepresentation, SemanticAction};
use crate::{PolicyStopAttestation, RepKey, Result};
use std::collections::{BTreeMap, HashSet};

pub const HIGH_U_POLICY_MANIFEST_VERSION: &str = "unknotdb-high-u-l1000-policy-v0";
pub const HIGH_U_BRANCH_MANIFEST_VERSION: &str = "unknotdb-high-u-one-cc-branch-v0";
pub const HIGH_U_TEMPLATE_MANIFEST_VERSION: &str = "unknotdb-high-u-anchored-template-v0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchoredTemplateProposal {
    pub template_id: u32,
    pub uses: u32,
    pub program: CheckpointedProofProgram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighUTemplateRun {
    pub selected: usize,
    pub improved: usize,
    pub evaluated_bindings: usize,
    pub legal_bindings: usize,
    pub exact_graph_hits: usize,
    pub inserted_edges: usize,
    pub manifest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HighUTemplateLimits {
    pub min_u: u32,
    pub cohort_limit: usize,
    pub max_templates: usize,
    pub max_anchors_per_node: usize,
}

/// Apply frequent relative proof templates at every bounded concrete anchor.
/// Templates are proposals only: exact materialization, replay, normalized
/// endpoint membership and strict U improvement are required before insertion.
pub fn improve_high_u_with_templates(
    graph: &mut PopulationGraph,
    templates: &[AnchoredTemplateProposal],
    snapshot_id: &str,
    selected_keys: Option<&[RepKey]>,
    limits: HighUTemplateLimits,
) -> Result<HighUTemplateRun> {
    let HighUTemplateLimits {
        min_u,
        cohort_limit,
        max_templates,
        max_anchors_per_node,
    } = limits;
    if snapshot_id.is_empty()
        || cohort_limit == 0
        || max_templates == 0
        || max_anchors_per_node == 0
    {
        return Err("anchored-template run limits must be positive".into());
    }
    let allowed = selected_keys.map(|keys| keys.iter().copied().collect::<HashSet<_>>());
    let mut selected = graph.ranked_quality_seeds(graph.node_count())?;
    selected.retain(|seed| {
        seed.u_upper_bound >= min_u
            && allowed
                .as_ref()
                .is_none_or(|keys| keys.contains(&seed.representation.key))
    });
    selected.truncate(cohort_limit);
    let templates = &templates[..templates.len().min(max_templates)];
    let before_edges = graph.edge_count();
    let mut improved = 0_usize;
    let mut evaluated_bindings = 0_usize;
    let mut legal_bindings = 0_usize;
    let mut exact_graph_hits = 0_usize;
    let mut manifest = format!(
        "{HIGH_U_TEMPLATE_MANIFEST_VERSION}\nsnapshot_id\t{snapshot_id}\nlimits\t{min_u}\t{cohort_limit}\t{}\t{max_anchors_per_node}\n",
        templates.len()
    );
    manifest.push_str(
        "result\tkey\told_u\tnew_u\ttemplate_id\tanchor_x\tanchor_y\tevaluated\tlegal\thits\tstatus\n",
    );
    for seed in selected.iter() {
        let old_u = graph
            .u_upper_bound(&seed.representation.key)
            .ok_or("template source disappeared")?;
        let source = &seed.representation.representation;
        let anchor_limit = source
            .word
            .len()
            .saturating_add(1)
            .min(max_anchors_per_node);
        let mut evaluated = 0_usize;
        let mut legal = 0_usize;
        let mut hits = 0_usize;
        let mut best: Option<(u32, u32, u16, u32, CheckpointedProofProgram, RepKey)> = None;
        for proposal in templates {
            if proposal.program.cc_cost() != 1 {
                continue;
            }
            for anchor_y in 0..anchor_limit {
                for anchor_x in 1..=source.strands {
                    evaluated += 1;
                    evaluated_bindings += 1;
                    let anchored = crate::representation::AnchoredProofProgram {
                        template: proposal.program.clone(),
                        anchor_x,
                        anchor_y: anchor_y.try_into()?,
                    };
                    let program = match anchored.materialize(source) {
                        Ok(program) => program,
                        Err(_) => continue,
                    };
                    let target = match program.replay(source) {
                        Ok(target) => target,
                        Err(_) => continue,
                    };
                    legal += 1;
                    legal_bindings += 1;
                    let normalized = target.normalize()?;
                    if normalized.representation != target {
                        continue;
                    }
                    let Some(target_u) = graph.u_upper_bound(&normalized.key) else {
                        continue;
                    };
                    hits += 1;
                    exact_graph_hits += 1;
                    let candidate = target_u.checked_add(1).ok_or("template U overflow")?;
                    let ordering = (candidate, proposal.template_id, anchor_x, anchor_y as u32);
                    if candidate < old_u
                        && best
                            .as_ref()
                            .is_none_or(|(u, id, x, y, _, _)| ordering < (*u, *id, *x, *y))
                    {
                        best = Some((
                            candidate,
                            proposal.template_id,
                            anchor_x,
                            anchor_y.try_into()?,
                            program,
                            normalized.key,
                        ));
                    }
                }
            }
        }
        let (new_u, template_id, anchor_x, anchor_y, status) =
            if let Some((_, template_id, anchor_x, anchor_y, program, target_key)) = best {
                graph.relax_unknot_edge_deferred_acs10(
                    seed.representation.clone(),
                    seed.policy_stop,
                    target_key,
                    program,
                    None,
                )?;
                graph.recompute_unknot_routes()?;
                let new_u = graph
                    .u_upper_bound(&seed.representation.key)
                    .ok_or("template source lost its route")?;
                if new_u < old_u {
                    improved += 1;
                    (
                        new_u,
                        template_id.to_string(),
                        anchor_x.to_string(),
                        anchor_y.to_string(),
                        "improved",
                    )
                } else {
                    (
                        new_u,
                        template_id.to_string(),
                        anchor_x.to_string(),
                        anchor_y.to_string(),
                        "no-strict-relaxation",
                    )
                }
            } else {
                (
                    old_u,
                    "-".into(),
                    "-".into(),
                    "-".into(),
                    "no-strict-known-target",
                )
            };
        manifest.push_str(&format!(
            "result\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            hex(&seed.representation.key),
            old_u,
            new_u,
            template_id,
            anchor_x,
            anchor_y,
            evaluated,
            legal,
            hits,
            status
        ));
    }
    graph.recompute_unknot_routes()?;
    graph.recompute_acs10_routes()?;
    let inserted_edges = graph.edge_count() - before_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\n",
        selected.len(),
        improved,
        evaluated_bindings,
        legal_bindings,
        exact_graph_hits,
        inserted_edges
    ));
    Ok(HighUTemplateRun {
        selected: selected.len(),
        improved,
        evaluated_bindings,
        legal_bindings,
        exact_graph_hits,
        inserted_edges,
        manifest,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighUBranchRun {
    pub selected: usize,
    pub improved: usize,
    pub excluded: usize,
    pub exact_graph_hits: usize,
    pub evaluated_branches: usize,
    pub collateral_improvements: usize,
    pub inserted_edges: usize,
    pub manifest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HighUBranchLimits {
    pub min_u: u32,
    pub cohort_limit: usize,
    pub max_branches_per_node: usize,
    pub max_depth: u16,
    pub policy_limits: PolicyLimits,
}

#[derive(Clone)]
struct BranchState {
    report: PreprocessingReport,
    edges: Vec<PolicyEdgeTrace>,
    positions: Vec<usize>,
    seen: HashSet<RepKey>,
}

/// Check every bounded one-CC choice at each selected stopping point, then run
/// the mandatory pinned preprocessing contract. Only exact hits in the
/// already-connected graph are eligible proof targets.
pub fn improve_high_u_one_cc_branches<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    oracle: &mut O,
    snapshot_id: &str,
    limits: HighUBranchLimits,
    selected_keys: Option<&[RepKey]>,
) -> Result<HighUBranchRun> {
    if snapshot_id.is_empty()
        || limits.cohort_limit == 0
        || limits.max_branches_per_node == 0
        || limits.max_depth == 0
    {
        return Err("one-CC branch limits and snapshot ID must be non-empty".into());
    }
    let before: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let before_edges = graph.edge_count();
    let allowed: Option<HashSet<_>> = selected_keys.map(|keys| keys.iter().copied().collect());
    let mut selected = graph.ranked_quality_seeds(graph.node_count())?;
    selected.retain(|seed| {
        seed.u_upper_bound >= limits.min_u
            && allowed
                .as_ref()
                .is_none_or(|set| set.contains(&seed.representation.key))
    });
    selected.truncate(limits.cohort_limit);
    let selected_keys: HashSet<_> = selected
        .iter()
        .map(|seed| seed.representation.key)
        .collect();
    let mut manifest = format!(
        "{HIGH_U_BRANCH_MANIFEST_VERSION}\nsnapshot_id\t{snapshot_id}\nmodel_id\t{}\nlimits\t{}\t{}\t{}\t{}\t{}\t{}\n",
        oracle.model_id(),
        limits.min_u,
        limits.cohort_limit,
        limits.max_branches_per_node,
        limits.max_depth,
        limits.policy_limits.max_policy_plies,
        limits.policy_limits.max_semantic_moves,
    );
    manifest.push_str(
        "result\tkey\told_u\tcurrent_u\tword_length\tevaluated\thits\tchosen_positions\tnew_u\tstatus\n",
    );
    let mut improved = 0_usize;
    let mut excluded = 0_usize;
    let mut exact_graph_hits = 0_usize;
    let mut evaluated_branches = 0_usize;
    for seed in &selected {
        let current_u = graph
            .u_upper_bound(&seed.representation.key)
            .ok_or("one-CC branch source disappeared")?;
        let report = preprocess_to_stopping_point(
            &seed.representation.representation,
            oracle,
            limits.policy_limits,
        )?;
        if !report.is_graph_stopping_point() || report.output.key != seed.representation.key {
            excluded += 1;
            manifest.push_str(&format!(
                "result\t{}\t{}\t{}\t{}\t0\t0\t-\t{}\treattestation-mismatch:{:?}\n",
                hex(&seed.representation.key),
                seed.u_upper_bound,
                current_u,
                seed.representation.representation.word.len(),
                current_u,
                report.stop_reason,
            ));
            continue;
        }
        let mut evaluated = 0_usize;
        let mut hits = 0_usize;
        let mut best: Option<(u32, Vec<usize>, Vec<PolicyEdgeTrace>)> = None;
        let mut frontier = vec![BranchState {
            report,
            edges: Vec::new(),
            positions: Vec::new(),
            seen: HashSet::from([seed.representation.key]),
        }];
        for depth in 1..=limits.max_depth {
            let mut next = Vec::new();
            for state in frontier {
                let branch_count = state
                    .report
                    .output
                    .representation
                    .word
                    .len()
                    .min(limits.max_branches_per_node);
                for position in 0..branch_count {
                    evaluated += 1;
                    evaluated_branches += 1;
                    let edge = match trace_crossing_change_edge_from_attestation(
                        state.report.clone(),
                        SemanticAction::CrossingChange {
                            position: position.try_into()?,
                        },
                        oracle,
                    ) {
                        Ok(edge) => edge,
                        Err(error)
                            if error.to_string().starts_with(
                                "successor preprocessing did not reach a graph stopping point",
                            ) =>
                        {
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    edge.verify()?;
                    if state.seen.contains(&edge.target.key) {
                        continue;
                    }
                    let mut edges = state.edges.clone();
                    edges.push(edge.clone());
                    let mut positions = state.positions.clone();
                    positions.push(position);
                    if let Some(target_u) = graph.u_upper_bound(&edge.target.key) {
                        hits += 1;
                        exact_graph_hits += 1;
                        let candidate_u = target_u
                            .checked_add(u32::from(depth))
                            .ok_or("branched U overflow")?;
                        if candidate_u < current_u
                            && best.as_ref().is_none_or(|(best_u, best_positions, _)| {
                                (candidate_u, positions.as_slice())
                                    < (*best_u, best_positions.as_slice())
                            })
                        {
                            best = Some((candidate_u, positions.clone(), edges.clone()));
                        }
                    }
                    if depth < limits.max_depth {
                        let mut seen = state.seen.clone();
                        seen.insert(edge.target.key);
                        next.push(BranchState {
                            report: edge.successor_preprocessing,
                            edges,
                            positions,
                            seen,
                        });
                    }
                }
            }
            frontier = next;
        }
        let mut new_u = current_u;
        let mut chosen = "-".to_owned();
        let status = if let Some((_, positions, edges)) = best {
            let mut outcomes = Vec::new();
            for edge in edges.into_iter().rev() {
                outcomes.push(graph.relax_unknot_edge_deferred_acs10(
                    edge.source,
                    attestation(&edge.source_attestation)?,
                    edge.target.key,
                    edge.program,
                    None,
                )?);
            }
            new_u = graph
                .u_upper_bound(&seed.representation.key)
                .ok_or("one-CC improved source disappeared")?;
            if new_u < current_u {
                improved += 1;
                chosen = positions
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("improved:{outcomes:?}")
            } else {
                format!("no-improvement:{outcomes:?}")
            }
        } else {
            "no-strict-known-target".into()
        };
        manifest.push_str(&format!(
            "result\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            hex(&seed.representation.key),
            seed.u_upper_bound,
            current_u,
            seed.representation.representation.word.len(),
            evaluated,
            hits,
            chosen,
            new_u,
            status,
        ));
    }
    graph.recompute_unknot_routes()?;
    graph.recompute_acs10_routes()?;
    let after: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let collateral_improvements = before
        .iter()
        .filter(|(key, old)| {
            !selected_keys.contains(*key) && after.get(*key).is_some_and(|new| new < *old)
        })
        .count();
    let inserted_edges = graph.edge_count() - before_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        selected.len(),
        improved,
        excluded,
        evaluated_branches,
        exact_graph_hits,
        collateral_improvements,
        inserted_edges,
    ));
    Ok(HighUBranchRun {
        selected: selected.len(),
        improved,
        excluded,
        exact_graph_hits,
        evaluated_branches,
        collateral_improvements,
        inserted_edges,
        manifest,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HighUPolicyLimits {
    pub min_u: u32,
    pub cohort_limit: usize,
    pub per_node_simulations: u32,
    pub total_simulations: u32,
    pub max_track_depth: u16,
    pub policy_limits: PolicyLimits,
}

impl HighUPolicyLimits {
    fn validate(self) -> Result<()> {
        if self.cohort_limit == 0
            || self.per_node_simulations == 0
            || self.total_simulations == 0
            || self.max_track_depth == 0
        {
            return Err("high-U cohort and search budgets must be positive".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighUNodeResult {
    pub source_key: RepKey,
    pub old_u: u32,
    pub before_search_u: u32,
    pub new_u: u32,
    pub simulations: u32,
    pub states: u32,
    pub policy_only_u: Option<u32>,
    pub graph_assisted_u: Option<u32>,
    pub chosen_depth: Option<u16>,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub inverse_witnesses: usize,
    pub improved: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighUPolicyRun {
    pub limits: HighUPolicyLimits,
    pub eligible_above_threshold: usize,
    pub selected: usize,
    pub excluded: usize,
    pub improved: usize,
    pub collateral_improvements: usize,
    pub total_simulations: u32,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub total_selected_u_decrease: u64,
    pub results: Vec<HighUNodeResult>,
    pub manifest: String,
}

#[derive(Clone)]
struct TrackState {
    stopping: NormalizedRepresentation,
    report: PreprocessingReport,
    edge_from_parent: Option<CheckpointedProofProgram>,
}

/// Follow the pinned policy's exact preferred CC trajectory for each selected
/// high-U node. Graph hits are only trusted as terminals with already verified
/// finite U. A staged track is committed only when it strictly improves the
/// selected source after global Bellman relaxation.
pub fn improve_high_u_with_policy<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    oracle: &mut O,
    snapshot_id: &str,
    limits: HighUPolicyLimits,
    selected_keys: Option<&[RepKey]>,
) -> Result<HighUPolicyRun> {
    limits.validate()?;
    if snapshot_id.is_empty() {
        return Err("high-U run snapshot ID must be non-empty".into());
    }
    let allowed: Option<HashSet<_>> = selected_keys.map(|keys| keys.iter().copied().collect());
    let eligible_above_threshold = graph
        .u_bounds()
        .into_iter()
        .filter(|(key, u)| {
            *u >= limits.min_u && allowed.as_ref().is_none_or(|set| set.contains(key))
        })
        .count();
    let mut selected = graph.ranked_quality_seeds(graph.node_count())?;
    selected.retain(|seed| {
        seed.u_upper_bound >= limits.min_u
            && allowed
                .as_ref()
                .is_none_or(|set| set.contains(&seed.representation.key))
    });
    selected.truncate(limits.cohort_limit);
    let selected_old: BTreeMap<_, _> = selected
        .iter()
        .map(|seed| (seed.representation.key, seed.u_upper_bound))
        .collect();
    let all_before: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let parent_nodes = graph.node_count();
    let parent_edges = graph.edge_count();
    let mut total_simulations = 0_u32;
    let mut excluded = 0_usize;
    let mut results = Vec::with_capacity(selected.len());
    let mut manifest = format!(
        "{HIGH_U_POLICY_MANIFEST_VERSION}\nsnapshot_id\t{snapshot_id}\nmodel_id\t{}\nlimits\t{}\t{}\t{}\t{}\t{}\t{}\t{}\neligible\t{}\n",
        oracle.model_id(),
        limits.min_u,
        limits.cohort_limit,
        limits.per_node_simulations,
        limits.total_simulations,
        limits.max_track_depth,
        limits.policy_limits.max_policy_plies,
        limits.policy_limits.max_semantic_moves,
        eligible_above_threshold,
    );
    manifest.push_str("selected\tordinal\tkey\told_u\tstrands\tword_length\tactive_witness_len\n");
    for (ordinal, seed) in selected.iter().enumerate() {
        manifest.push_str(&format!(
            "selected\t{}\t{}\t{}\t{}\t{}\t{}\n",
            ordinal,
            hex(&seed.representation.key),
            seed.u_upper_bound,
            seed.representation.representation.strands,
            seed.representation.representation.word.len(),
            seed.active_witness_semantic_len,
        ));
    }

    for seed in &selected {
        let old_u = seed.u_upper_bound;
        let before_search_u = graph
            .u_upper_bound(&seed.representation.key)
            .ok_or("selected high-U source disappeared")?;
        let remaining_total = limits.total_simulations.saturating_sub(total_simulations);
        if remaining_total == 0 {
            results.push(no_improvement(
                seed.representation.key,
                old_u,
                before_search_u,
                "total simulation budget exhausted",
            ));
            continue;
        }
        let item_budget = remaining_total.min(limits.per_node_simulations);
        let initial = preprocess_to_stopping_point(
            &seed.representation.representation,
            oracle,
            limits.policy_limits,
        )?;
        if !initial.is_graph_stopping_point() || initial.output.key != seed.representation.key {
            excluded += 1;
            results.push(no_improvement(
                seed.representation.key,
                old_u,
                before_search_u,
                &format!("source reattestation mismatch: {:?}", initial.stop_reason),
            ));
            continue;
        }
        let mut states = vec![TrackState {
            stopping: initial.output.clone(),
            report: initial,
            edge_from_parent: None,
        }];
        let mut seen = HashSet::from([seed.representation.key]);
        let mut simulations = 0_u32;
        let mut policy_only_u = None;
        let mut best_graph: Option<(u32, usize)> = None;
        let mut track_failure = None;
        while simulations < item_budget && simulations < u32::from(limits.max_track_depth) {
            let current = states.last().ok_or("empty high-U policy track")?;
            if current.report.stop_reason == PolicyStopReason::TerminalRepresentation {
                policy_only_u = Some(simulations);
                break;
            }
            let edge = match trace_policy_edge_from_attestation(current.report.clone(), oracle) {
                Ok(edge) => edge,
                Err(error)
                    if error.to_string().starts_with(
                        "successor preprocessing did not reach a graph stopping point",
                    ) =>
                {
                    track_failure = Some(error.to_string());
                    break;
                }
                Err(error) => return Err(error),
            };
            edge.verify()?;
            simulations += 1;
            if !seen.insert(edge.target.key) {
                break;
            }
            states.push(TrackState {
                stopping: edge.target.clone(),
                report: edge.successor_preprocessing,
                edge_from_parent: Some(edge.program),
            });
            let index = states.len() - 1;
            if let Some(target_u) = graph.u_upper_bound(&states[index].stopping.key) {
                let proposed = target_u
                    .checked_add(simulations)
                    .ok_or("high-U proposed bound overflow")?;
                if best_graph.is_none_or(|(best, _)| proposed < best) {
                    best_graph = Some((proposed, index));
                }
            }
            if states[index].report.stop_reason == PolicyStopReason::TerminalRepresentation {
                policy_only_u = Some(simulations);
                break;
            }
        }
        total_simulations = total_simulations
            .checked_add(simulations)
            .ok_or("high-U total simulation overflow")?;
        let graph_assisted_u = best_graph.map(|(u, _)| u);
        let policy_index = policy_only_u.map(|_| states.len() - 1);
        let chosen = match (best_graph, policy_only_u.zip(policy_index)) {
            (Some((graph_u, graph_index)), Some((policy_u, policy_index))) => {
                if policy_u < graph_u {
                    Some((policy_u, policy_index))
                } else {
                    Some((graph_u, graph_index))
                }
            }
            (Some(candidate), None) => Some(candidate),
            (None, Some(candidate)) => Some(candidate),
            (None, None) => None,
        };
        let old_nodes = graph.node_count();
        let old_edges = graph.edge_count();
        let mut inserted_nodes = 0_usize;
        let mut inserted_edges = 0_usize;
        let mut inverse_witnesses = 0_usize;
        let mut new_u = before_search_u;
        let mut reason = track_failure
            .unwrap_or_else(|| "no finite graph/policy terminal within budget".to_owned());
        if let Some((proposed, terminal_index)) = chosen {
            if proposed < before_search_u {
                let mut staged = graph.clone();
                for index in (1..=terminal_index).rev() {
                    let program = states[index]
                        .edge_from_parent
                        .clone()
                        .ok_or("high-U track edge is absent")?;
                    let inverse =
                        program.invert_edge(&states[index - 1].stopping.representation)?;
                    if inverse.program.replay(&inverse.source)? != inverse.target {
                        return Err("high-U inverse witness failed exact replay".into());
                    }
                    inverse_witnesses += 1;
                    staged.relax_unknot_edge_deferred_acs10(
                        states[index - 1].stopping.clone(),
                        attestation(&states[index - 1].report)?,
                        states[index].stopping.key,
                        program,
                        None,
                    )?;
                }
                staged.recompute_unknot_routes()?;
                staged.recompute_acs10_routes()?;
                let candidate_u = staged
                    .u_upper_bound(&seed.representation.key)
                    .ok_or("staged high-U source lost its route")?;
                if candidate_u < before_search_u {
                    *graph = staged;
                    new_u = candidate_u;
                    inserted_nodes = graph.node_count() - old_nodes;
                    inserted_edges = graph.edge_count() - old_edges;
                    reason = "strict verified improvement".into();
                } else {
                    reason = "candidate track did not survive global strict relaxation".into();
                }
            } else {
                reason = format!("best verified proposal {proposed} is not below current U");
            }
        }
        results.push(HighUNodeResult {
            source_key: seed.representation.key,
            old_u,
            before_search_u,
            new_u,
            simulations,
            states: states
                .len()
                .try_into()
                .map_err(|_| "too many high-U states")?,
            policy_only_u,
            graph_assisted_u,
            chosen_depth: chosen.map(|(_, index)| index as u16),
            inserted_nodes,
            inserted_edges,
            inverse_witnesses,
            improved: new_u < before_search_u,
            reason,
        });
    }
    graph.recompute_unknot_routes()?;
    graph.recompute_acs10_routes()?;
    let final_u: BTreeMap<_, _> = graph.u_bounds().into_iter().collect();
    let improved = results.iter().filter(|result| result.improved).count();
    let selected_keys: HashSet<_> = selected_old.keys().copied().collect();
    let collateral_improvements = all_before
        .iter()
        .filter(|(key, old)| {
            !selected_keys.contains(*key) && final_u.get(*key).is_some_and(|new| new < *old)
        })
        .count();
    let total_selected_u_decrease = selected_old
        .iter()
        .map(|(key, old)| u64::from(old.saturating_sub(final_u[key])))
        .sum();
    for result in &results {
        manifest.push_str(&format!(
            "result\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            hex(&result.source_key),
            result.old_u,
            result.before_search_u,
            result.new_u,
            result.simulations,
            result.states,
            optional_u32(result.policy_only_u),
            optional_u32(result.graph_assisted_u),
            result
                .chosen_depth
                .map_or_else(|| "-".into(), |v| v.to_string()),
            result.inserted_nodes,
            result.inserted_edges,
            result.inverse_witnesses,
            u8::from(result.improved),
            result.reason,
        ));
    }
    let inserted_nodes = graph.node_count() - parent_nodes;
    let inserted_edges = graph.edge_count() - parent_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        selected.len(),
        excluded,
        improved,
        collateral_improvements,
        total_simulations,
        inserted_nodes,
        inserted_edges,
        total_selected_u_decrease,
        eligible_above_threshold,
    ));
    Ok(HighUPolicyRun {
        limits,
        eligible_above_threshold,
        selected: selected.len(),
        excluded,
        improved,
        collateral_improvements,
        total_simulations,
        inserted_nodes,
        inserted_edges,
        total_selected_u_decrease,
        results,
        manifest,
    })
}

fn attestation(report: &PreprocessingReport) -> Result<PolicyStopAttestation> {
    let audit_sha256 = preprocessing_audit_sha256(report)?;
    match report.stop_reason {
        PolicyStopReason::PreferredCrossingChange => {
            Ok(PolicyStopAttestation::PreferredCrossingChange {
                action: report
                    .preferred_crossing_change
                    .ok_or("high-U CC stop lacks action")?
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
        _ => Err("incomplete high-U preprocessing has no attestation".into()),
    }
}

fn no_improvement(key: RepKey, old_u: u32, current_u: u32, reason: &str) -> HighUNodeResult {
    HighUNodeResult {
        source_key: key,
        old_u,
        before_search_u: current_u,
        new_u: current_u,
        simulations: 0,
        states: 0,
        policy_only_u: None,
        graph_assisted_u: None,
        chosen_depth: None,
        inserted_nodes: 0,
        inserted_edges: 0,
        inverse_witnesses: 0,
        improved: false,
        reason: reason.to_owned(),
    }
}

fn optional_u32(value: Option<u32>) -> String {
    value.map_or_else(|| "-".into(), |value| value.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
