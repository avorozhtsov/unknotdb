//! Fixed-budget, target-gated braid search with one persistent policy oracle.

use crate::policy::{PolicyLimits, PolicyOracle};
use crate::population::PopulationGraph;
use crate::representation::BraidRepresentation;
use crate::targeted::{force_connected_insert, ConnectedInsertDisposition, ConnectedInsertLimits};
use crate::Result;
use std::fs;
use std::path::Path;

pub const TARGET_CAMPAIGN_VERSION: &str = "unknotdb-targeted-braid-campaign-v0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedBraidEntry {
    pub representation_id: String,
    pub target_u: u32,
    pub representation: BraidRepresentation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetCampaignLimits {
    pub per_item_simulations: u32,
    pub total_simulations: u32,
    pub max_track_depth: u16,
    pub max_macro_semantic_depth: u16,
    pub max_macro_states: u32,
    pub policy_limits: PolicyLimits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetCampaignRun {
    pub selected: usize,
    pub attempted: usize,
    pub already_satisfied: usize,
    pub improved_to_target: usize,
    pub misses: usize,
    pub simulations: u32,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub manifest: String,
}

pub fn read_targeted_braids_tsv(path: &Path) -> Result<Vec<TargetedBraidEntry>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    if lines.next() != Some("representation_id\ttarget_u\tstrands\tword") {
        return Err("targeted braid TSV has an unsupported header".into());
    }
    let mut entries = Vec::new();
    for (line_index, line) in lines.enumerate() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 4 {
            return Err(format!("targeted braid line {} is malformed", line_index + 2).into());
        }
        let word = if fields[3].is_empty() {
            Vec::new()
        } else {
            fields[3]
                .split(',')
                .map(str::parse::<i16>)
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let representation = BraidRepresentation {
            strands: fields[2].parse()?,
            cyclic_band_generators: false,
            word,
        };
        representation.validate()?;
        if !representation.is_knot_closure()? {
            return Err(format!("targeted braid line {} is not a knot", line_index + 2).into());
        }
        entries.push(TargetedBraidEntry {
            representation_id: fields[0].to_owned(),
            target_u: fields[1].parse()?,
            representation,
        });
    }
    Ok(entries)
}

pub fn run_target_campaign<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    entries: &[TargetedBraidEntry],
    oracle: &mut O,
    snapshot_id: &str,
    cohort_id: &str,
    limits: TargetCampaignLimits,
) -> Result<TargetCampaignRun> {
    if limits.per_item_simulations == 0
        || limits.total_simulations == 0
        || limits.max_track_depth == 0
        || limits.max_macro_semantic_depth == 0
        || limits.max_macro_states == 0
        || snapshot_id.is_empty()
        || cohort_id.is_empty()
    {
        return Err("target campaign identifiers and budgets must be positive".into());
    }
    let parent_nodes = graph.node_count();
    let parent_edges = graph.edge_count();
    let mut attempted = 0_usize;
    let mut already_satisfied = 0_usize;
    let mut improved_to_target = 0_usize;
    let mut misses = 0_usize;
    let mut simulations = 0_u32;
    let mut manifest = format!(
        "{TARGET_CAMPAIGN_VERSION}\nsnapshot_id\t{snapshot_id}\ncohort_id\t{cohort_id}\nmodel_id\t{}\nlimits\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        oracle.model_id(),
        limits.per_item_simulations,
        limits.total_simulations,
        limits.max_track_depth,
        limits.max_macro_semantic_depth,
        limits.max_macro_states,
        limits.policy_limits.max_policy_plies,
        limits.policy_limits.max_semantic_moves,
    );
    manifest.push_str("item\trepresentation_id\ttarget_u\tstatus\tresult_u\tsimulations\n");
    for entry in entries {
        let remaining = limits.total_simulations.saturating_sub(simulations);
        if remaining == 0 {
            break;
        }
        let budget = remaining.min(limits.per_item_simulations);
        let before = entry
            .representation
            .normalize()
            .ok()
            .and_then(|normalized| graph.u_upper_bound(&normalized.key));
        let result = force_connected_insert(
            graph,
            &entry.representation,
            oracle,
            ConnectedInsertLimits {
                max_simulations: budget,
                max_search_states: budget.saturating_add(1),
                max_track_depth: limits.max_track_depth,
                max_result_u: Some(entry.target_u),
                max_macro_semantic_depth: limits.max_macro_semantic_depth,
                max_macro_states: limits.max_macro_states,
                policy_limits: limits.policy_limits,
            },
            &entry.representation_id,
            snapshot_id,
        )?;
        attempted += 1;
        simulations = simulations
            .checked_add(result.simulations)
            .ok_or("target campaign simulation overflow")?;
        let (status, result_u) = match result.disposition {
            ConnectedInsertDisposition::AlreadyCovered { u_upper_bound } => {
                already_satisfied += 1;
                ("already-satisfied", Some(u_upper_bound))
            }
            ConnectedInsertDisposition::Inserted { u_upper_bound } => {
                if u_upper_bound > entry.target_u {
                    return Err("target campaign admitted a route above its hard target".into());
                }
                improved_to_target += 1;
                ("improved-to-target", Some(u_upper_bound))
            }
            ConnectedInsertDisposition::CoverageMiss { .. } => {
                misses += 1;
                ("miss", before)
            }
        };
        manifest.push_str(&format!(
            "item\t{}\t{}\t{}\t{}\t{}\n",
            entry.representation_id,
            entry.target_u,
            status,
            result_u
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".into()),
            result.simulations,
        ));
        manifest.push_str("detail_begin\n");
        manifest.push_str(&result.manifest);
        manifest.push_str("detail_end\n");
    }
    let inserted_nodes = graph.node_count() - parent_nodes;
    let inserted_edges = graph.edge_count() - parent_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        entries.len(),
        attempted,
        already_satisfied,
        improved_to_target,
        misses,
        simulations,
        inserted_nodes,
        inserted_edges,
    ));
    Ok(TargetCampaignRun {
        selected: entries.len(),
        attempted,
        already_satisfied,
        improved_to_target,
        misses,
        simulations,
        inserted_nodes,
        inserted_edges,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{OracleDecision, CONTROLLER_INITIAL_STATE, GRAPH_STOP_OBJECTIVE_RATIO};

    struct PreferredCcOracle;
    impl PolicyOracle for PreferredCcOracle {
        fn model_id(&self) -> &str {
            "fake:Q254:target-campaign"
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
            Ok(OracleDecision::PreferredCrossingChange {
                action: crate::representation::SemanticAction::CrossingChange { position: 0 },
                controller_plies: 1,
            })
        }
    }

    #[test]
    fn campaign_admits_only_routes_within_the_row_target() {
        let mut graph = PopulationGraph::from_unknot([7; 32]).unwrap();
        let entries = vec![TargetedBraidEntry {
            representation_id: "trefoil".into(),
            target_u: 1,
            representation: BraidRepresentation {
                strands: 2,
                cyclic_band_generators: false,
                word: vec![1, 1, 1],
            },
        }];
        let run = run_target_campaign(
            &mut graph,
            &entries,
            &mut PreferredCcOracle,
            "snapshot",
            "cohort",
            TargetCampaignLimits {
                per_item_simulations: 8,
                total_simulations: 8,
                max_track_depth: 1,
                max_macro_semantic_depth: 1,
                max_macro_states: 1,
                policy_limits: PolicyLimits::default(),
            },
        )
        .unwrap();
        assert_eq!(run.improved_to_target, 1);
        assert_eq!(run.misses, 0);
    }
}
