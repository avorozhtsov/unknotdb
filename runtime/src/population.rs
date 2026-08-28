//! Monotone reverse-DP population around the unknot.
//!
//! A candidate macro is accepted only after exact replay. Relaxation then uses
//! `U(source) <= cc(macro) + U(target)`. Previously accepted edges remain as
//! evidence when a better route replaces the materialized pointer.

use crate::representation::{
    verify_edge_program_with_certificate, BraidRepresentation, CheckpointedProofProgram,
    ProofInstruction, SemanticAction, VALIDATOR_VERSION,
};
use crate::{
    EdgeRecord, GraphSnapshot, NodeId, NodeRecord, PolicyStopAttestation, RepKey, Result,
    RouteStep, SnapshotMeta, SqliteSnapshot, ROLE_CORE,
};
use rusqlite::OptionalExtension;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
struct KnowledgeNode {
    representation: crate::representation::NormalizedRepresentation,
    policy_stop: PolicyStopAttestation,
    u_upper_bound: u32,
    route_rank: u64,
    active_unknot_edge: Option<usize>,
    acs_route_rank: u64,
    active_acs10_edge: Option<usize>,
}

#[derive(Clone, Debug)]
struct KnowledgeEdge {
    source_key: RepKey,
    target_key: RepKey,
    cc_cost: u8,
    first_action: u64,
    program: Vec<u8>,
    certificate_id: Option<RepKey>,
    certificate: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct EdgeIdentity {
    source_key: RepKey,
    target_key: RepKey,
    cc_cost: u8,
    first_action: u64,
    program_hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelaxationOutcome {
    Inserted { u_upper_bound: u32 },
    Improved { old: u32, new: u32 },
    AcceptedAcs10 { current: u32, proposed: u32 },
    Unchanged { current: u32, proposed: u32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Acs10Refresh {
    pub eligible_edges: usize,
    pub active_routes: usize,
    pub changed_routes: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnknotRefresh {
    pub changed_bounds: usize,
    pub changed_routes: usize,
    pub reachable_nodes: usize,
}

#[derive(Clone, Debug)]
pub struct PopulationGraph {
    nodes: HashMap<RepKey, KnowledgeNode>,
    edges: Vec<KnowledgeEdge>,
    /// Builder-only exact-dedup index. Hash collisions retain a short bucket
    /// and are resolved by comparing the complete program bytes.
    edge_lookup: HashMap<EdgeIdentity, Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopulationSeed {
    pub representation: crate::representation::NormalizedRepresentation,
    pub u_upper_bound: u32,
    pub route_rank: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionSeed {
    pub representation: crate::representation::NormalizedRepresentation,
    pub policy_stop: PolicyStopAttestation,
    pub u_upper_bound: u32,
    pub route_rank: u64,
    pub active_witness_semantic_len: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopulationUpdateBenchmark {
    pub load_time: Duration,
    pub active_unknot_edges: usize,
    pub duplicate_relaxations: usize,
    pub duplicate_time: Duration,
    pub refresh_rounds: usize,
    pub refresh_time: Duration,
    pub rebuild_rounds: usize,
    pub rebuild_relaxations: usize,
    pub rebuild_time: Duration,
    pub batched_rebuild_relaxations: usize,
    pub batched_rebuild_time: Duration,
}

#[derive(Clone)]
struct RelaxationCase {
    route_rank: u64,
    source: crate::representation::NormalizedRepresentation,
    source_stop: PolicyStopAttestation,
    target_key: RepKey,
    program: CheckpointedProofProgram,
    certificate_id: Option<RepKey>,
    certificate: Option<Vec<u8>>,
}

impl PopulationGraph {
    /// Re-attest a conservative policy-adapter migration without rerunning the
    /// network. This is sound only when the new adapter differs exclusively in
    /// behavior after a controller/representation cycle: every stored graph
    /// node is already attested as terminal or as having reached a preferred
    /// CC under the old adapter, so the new fallback branch is unreachable on
    /// those exact clean-controller executions.
    pub fn migrate_conservative_policy_adapter(
        &mut self,
        old_adapter: &str,
        new_adapter: &str,
    ) -> (usize, usize) {
        let mut terminal = 0_usize;
        let mut preferred_cc = 0_usize;
        for node in self.nodes.values_mut() {
            let (kind, action, old_audit) = match node.policy_stop {
                PolicyStopAttestation::Terminal { audit_sha256 } => {
                    terminal += 1;
                    (b'T', None, audit_sha256)
                }
                PolicyStopAttestation::PreferredCrossingChange {
                    action,
                    audit_sha256,
                } => {
                    preferred_cc += 1;
                    (b'C', Some(action), audit_sha256)
                }
                PolicyStopAttestation::CapacityFallback { audit_sha256 } => {
                    (b'R', None, audit_sha256)
                }
            };
            let mut material =
                Vec::with_capacity(64 + old_adapter.len() + new_adapter.len() + old_audit.len());
            material.extend_from_slice(b"UNKNOTDB_CONSERVATIVE_POLICY_ADAPTER_MIGRATION_V0\0");
            material.push(kind);
            material.extend_from_slice(&action.unwrap_or(0).to_le_bytes());
            material.extend_from_slice(&old_audit);
            material.extend_from_slice(old_adapter.as_bytes());
            material.push(0);
            material.extend_from_slice(new_adapter.as_bytes());
            let audit_sha256 = unknotdb::util::sha256(&material);
            node.policy_stop = match (kind, action) {
                (b'C', Some(action)) => PolicyStopAttestation::PreferredCrossingChange {
                    action,
                    audit_sha256,
                },
                (b'T', None) => PolicyStopAttestation::Terminal { audit_sha256 },
                (b'R', None) => PolicyStopAttestation::CapacityFallback { audit_sha256 },
                _ => unreachable!("policy stop kind/action constructed above"),
            };
        }
        (terminal, preferred_cc)
    }

    pub fn from_unknot(audit_sha256: RepKey) -> Result<Self> {
        let unknot = BraidRepresentation {
            strands: 1,
            cyclic_band_generators: false,
            word: vec![],
        }
        .normalize()?;
        let node = KnowledgeNode {
            representation: unknot.clone(),
            policy_stop: PolicyStopAttestation::Terminal { audit_sha256 },
            u_upper_bound: 0,
            route_rank: 0,
            active_unknot_edge: None,
            acs_route_rank: 0,
            active_acs10_edge: None,
        };
        Ok(Self {
            nodes: HashMap::from([(unknot.key, node)]),
            edges: Vec::new(),
            edge_lookup: HashMap::new(),
        })
    }

    /// Resume a population build from a fully validated immutable snapshot.
    /// All hot/cold invariants and program hashes are checked before any row is
    /// admitted back into the mutable in-memory builder.
    pub fn from_snapshot(path: impl AsRef<Path>) -> Result<(Self, SnapshotMeta)> {
        let snapshot = SqliteSnapshot::open_file(path)?;
        snapshot.verify_program_hashes()?;
        let hot = GraphSnapshot::load_connection(snapshot.connection())?;
        if hot.meta().synthetic {
            return Err("cannot resume population from a synthetic snapshot".into());
        }
        let meta = hot.meta().clone();
        let conn = snapshot.connection();
        if snapshot.schema_version()? >= 4 {
            let attested: i64 =
                conn.query_row("SELECT count(*) FROM policy_stops", [], |row| row.get(0))?;
            let total: i64 = conn.query_row("SELECT count(*) FROM nodes", [], |row| row.get(0))?;
            if attested != total {
                return Err(
                    "mutable PopulationGraph resume does not yet support exact checkpoint vertices"
                        .into(),
                );
            }
        }
        let mut keys_by_node = Vec::with_capacity(hot.node_count());
        let mut nodes = HashMap::with_capacity(hot.node_count());
        let node_sql = if snapshot.schema_version()? >= 2 {
            "SELECT n.node_id, k.rep_key, n.u_upper_bound, n.unknot_route_rank, \
                    n.next_unknot_edge, n.acs_route_rank, n.next_acs10_edge, \
                    r.encoding, s.stop_kind, \
                    s.preferred_cc_action, s.audit_sha256 \
             FROM nodes n \
             JOIN node_keys k ON k.node_id=n.node_id \
             JOIN representations r ON r.node_id=n.node_id \
             JOIN policy_stops s ON s.node_id=n.node_id \
             ORDER BY n.node_id"
        } else {
            "SELECT n.node_id, n.rep_key, n.u_upper_bound, n.unknot_route_rank, \
                    n.next_unknot_edge, n.acs_route_rank, n.next_acs10_edge, \
                    r.encoding, s.stop_kind, \
                    s.preferred_cc_action, s.audit_sha256 \
             FROM nodes n \
             JOIN representations r ON r.node_id=n.node_id \
             JOIN policy_stops s ON s.node_id=n.node_id \
             ORDER BY n.node_id"
        };
        let mut stmt = conn.prepare(node_sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Vec<u8>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Vec<u8>>(10)?,
            ))
        })?;
        for row in rows {
            let (
                node_id,
                raw_key,
                u,
                rank,
                active,
                acs_rank,
                active_acs,
                encoding,
                stop_kind,
                action,
                raw_audit,
            ) = row?;
            let expected: i64 = keys_by_node
                .len()
                .try_into()
                .map_err(|_| "too many snapshot nodes")?;
            if node_id != expected {
                return Err("resumed population node IDs are not dense".into());
            }
            let key = key_from_blob(&raw_key, "node rep_key")?;
            let decoded = BraidRepresentation::decode_storage(&encoding)?;
            let normalized = decoded.normalize()?;
            if normalized.key != key || normalized.representation != decoded {
                return Err(format!("resumed node {} is not canonically keyed", node_id).into());
            }
            let audit_sha256 = key_from_blob(&raw_audit, "policy audit")?;
            let policy_stop = match (stop_kind, action) {
                (0, Some(action)) => PolicyStopAttestation::PreferredCrossingChange {
                    action: nonnegative_u64(action, "preferred CC")?,
                    audit_sha256,
                },
                (1, None) => PolicyStopAttestation::Terminal { audit_sha256 },
                (2, None) => PolicyStopAttestation::CapacityFallback { audit_sha256 },
                _ => {
                    return Err(
                        format!("resumed node {} has a malformed policy stop", node_id).into(),
                    )
                }
            };
            let active_unknot_edge = active
                .map(|value| nonnegative_usize(value, "next_unknot_edge"))
                .transpose()?;
            let active_acs10_edge = active_acs
                .map(|value| nonnegative_usize(value, "next_acs10_edge"))
                .transpose()?;
            let u_upper_bound: u32 = u
                .ok_or("resumed population node has no U upper bound")?
                .try_into()
                .map_err(|_| "resumed U upper bound does not fit u32")?;
            let route_rank: u64 = rank
                .ok_or("resumed population node has no unknot route rank")?
                .try_into()
                .map_err(|_| "resumed route rank is negative")?;
            let acs_route_rank: u64 = acs_rank
                .ok_or("resumed population node has no ACS10 route rank")?
                .try_into()
                .map_err(|_| "resumed ACS10 route rank is negative")?;
            keys_by_node.push(key);
            nodes.insert(
                key,
                KnowledgeNode {
                    representation: normalized,
                    policy_stop,
                    u_upper_bound,
                    route_rank,
                    active_unknot_edge,
                    acs_route_rank,
                    active_acs10_edge,
                },
            );
        }

        let mut edges = Vec::with_capacity(hot.edge_count());
        let schema: u32 = conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )?
            .parse()?;
        let legacy = schema == 0;
        let edge_sql = if legacy {
            "SELECT edge_id, source_node, target_node, cc_cost, first_action, \
                    program_version, program, certificate_id, validator_version,0,0 \
             FROM edges ORDER BY edge_id"
        } else if schema < 3 {
            "SELECT e.edge_id, e.source_node, e.target_node, e.cc_cost, e.first_action, \
                    p.program_version, p.program, e.certificate_id, '',0,0 \
             FROM edges e JOIN programs p ON p.program_id=e.program_id ORDER BY e.edge_id"
        } else {
            "SELECT e.edge_id, e.source_node, e.target_node, e.cc_cost, e.first_action, \
                    p.program_version, p.program, e.certificate_id, '', \
                    e.program_anchor_x, e.program_anchor_y \
             FROM edges e JOIN programs p ON p.program_id=e.program_id ORDER BY e.edge_id"
        };
        let mut stmt = conn.prepare(edge_sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Vec<u8>>(6)?,
                row.get::<_, Option<Vec<u8>>>(7)?,
                if legacy {
                    row.get::<_, String>(8)?
                } else {
                    meta.validator_version.clone()
                },
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
            ))
        })?;
        for row in rows {
            let (
                edge_id,
                source,
                target,
                cc,
                action,
                version,
                program,
                certificate,
                validator,
                anchor_x,
                anchor_y,
            ) = row?;
            if nonnegative_usize(edge_id, "edge_id")? != edges.len() {
                return Err("resumed population edge IDs are not dense".into());
            }
            if version != i64::from(CheckpointedProofProgram::VERSION)
                || validator != VALIDATOR_VERSION
            {
                return Err(format!(
                    "resumed edge {} uses an unsupported proof contract",
                    edge_id
                )
                .into());
            }
            let source = nonnegative_usize(source, "source_node")?;
            let target = nonnegative_usize(target, "target_node")?;
            let source_key = *keys_by_node
                .get(source)
                .ok_or("resumed edge source is absent")?;
            let program = if schema >= 3 {
                let template = CheckpointedProofProgram::decode(&program)?;
                if template.contains_planar_certificate() {
                    if anchor_x != 0 || anchor_y != 0 {
                        return Err("resumed planar program has a non-zero anchor".into());
                    }
                    template.encode()?
                } else {
                    crate::representation::AnchoredProofProgram {
                        template,
                        anchor_x: anchor_x
                            .try_into()
                            .map_err(|_| "resumed program anchor_x does not fit u16")?,
                        anchor_y: anchor_y
                            .try_into()
                            .map_err(|_| "resumed program anchor_y does not fit u32")?,
                    }
                    .materialize(
                        &nodes
                            .get(&source_key)
                            .ok_or("resumed edge source knowledge is absent")?
                            .representation
                            .representation,
                    )?
                    .encode()?
                }
            } else {
                program
            };
            let certificate_id = certificate
                .as_deref()
                .map(|blob| key_from_blob(blob, "certificate_id"))
                .transpose()?;
            let certificate = certificate_id
                .map(|id| -> Result<Option<Vec<u8>>> {
                    let exists = conn
                        .query_row(
                            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='planar_certificates'",
                            [],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some();
                    if !exists {
                        return Ok(None);
                    }
                    Ok(conn
                        .query_row(
                            "SELECT certificate FROM planar_certificates WHERE certificate_id=?1",
                            [id.as_slice()],
                            |row| row.get(0),
                        )
                        .optional()?)
                })
                .transpose()?
                .flatten();
            edges.push(KnowledgeEdge {
                source_key,
                target_key: *keys_by_node
                    .get(target)
                    .ok_or("resumed edge target is absent")?,
                cc_cost: cc
                    .try_into()
                    .map_err(|_| "resumed edge CC cost does not fit u8")?,
                first_action: nonnegative_u64(action, "edge first action")?,
                program,
                certificate_id,
                certificate,
            });
        }
        for node in nodes.values() {
            if node
                .active_unknot_edge
                .is_some_and(|edge_index| edge_index >= edges.len())
                || node
                    .active_acs10_edge
                    .is_some_and(|edge_index| edge_index >= edges.len())
            {
                return Err("resumed node points outside the edge arena".into());
            }
        }
        let edge_lookup = build_edge_lookup(&edges);
        Ok((
            Self {
                nodes,
                edges,
                edge_lookup,
            },
            meta,
        ))
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn planar_certificate_count(&self) -> usize {
        self.edges
            .iter()
            .filter_map(|edge| edge.certificate_id)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Measure proof replay, full ACS10 refresh, and true monotone insertions
    /// using the active unknot routes of one validated production snapshot.
    pub fn benchmark_updates(
        path: impl AsRef<Path>,
        duplicate_rounds: usize,
        refresh_rounds: usize,
        rebuild_rounds: usize,
    ) -> Result<PopulationUpdateBenchmark> {
        if duplicate_rounds == 0 || refresh_rounds == 0 || rebuild_rounds == 0 {
            return Err("population benchmark rounds must all be positive".into());
        }
        let load_started = Instant::now();
        let (mut graph, _) = Self::from_snapshot(path)?;
        let load_time = load_started.elapsed();
        let unknot_key = graph.unknot_key()?;
        let terminal_audit = match graph
            .nodes
            .get(&unknot_key)
            .ok_or("population benchmark has no unknot root")?
            .policy_stop
        {
            PolicyStopAttestation::Terminal { audit_sha256 } => audit_sha256,
            PolicyStopAttestation::PreferredCrossingChange { .. } => {
                return Err("population benchmark root is not terminal".into())
            }
            PolicyStopAttestation::CapacityFallback { .. } => {
                return Err("population benchmark root is a capacity fallback".into())
            }
        };
        let mut cases = Vec::with_capacity(graph.nodes.len().saturating_sub(1));
        for node in graph.nodes.values() {
            let Some(edge_index) = node.active_unknot_edge else {
                if node.representation.key != unknot_key {
                    return Err("population benchmark node has no active unknot edge".into());
                }
                continue;
            };
            let edge = graph
                .edges
                .get(edge_index)
                .ok_or("population benchmark active edge is absent")?;
            cases.push(RelaxationCase {
                route_rank: node.route_rank,
                source: node.representation.clone(),
                source_stop: node.policy_stop,
                target_key: edge.target_key,
                program: CheckpointedProofProgram::decode(&edge.program)?,
                certificate_id: edge.certificate_id,
                certificate: edge.certificate.clone(),
            });
        }
        cases.sort_by_key(|case| (case.route_rank, case.source.key));

        let duplicate_started = Instant::now();
        for _ in 0..duplicate_rounds {
            for case in &cases {
                let outcome = graph.relax_unknot_edge_impl(
                    case.source.clone(),
                    case.source_stop,
                    case.target_key,
                    case.program.clone(),
                    case.certificate_id,
                    case.certificate.clone(),
                    true,
                )?;
                if !matches!(outcome, RelaxationOutcome::Unchanged { .. }) {
                    return Err("duplicate population relaxation changed the graph".into());
                }
            }
        }
        let duplicate_time = duplicate_started.elapsed();

        let refresh_started = Instant::now();
        for _ in 0..refresh_rounds {
            let refresh = graph.recompute_acs10_routes()?;
            if refresh.changed_routes != 0 {
                return Err("ACS10 benchmark refresh changed a validated route overlay".into());
            }
        }
        let refresh_time = refresh_started.elapsed();

        let rebuild_started = Instant::now();
        for _ in 0..rebuild_rounds {
            let mut rebuilt = Self::from_unknot(terminal_audit)?;
            for case in &cases {
                let outcome = rebuilt.relax_unknot_edge_impl(
                    case.source.clone(),
                    case.source_stop,
                    case.target_key,
                    case.program.clone(),
                    case.certificate_id,
                    case.certificate.clone(),
                    true,
                )?;
                if !matches!(outcome, RelaxationOutcome::Inserted { .. }) {
                    return Err("population rebuild relaxation did not insert its source".into());
                }
            }
            if rebuilt.node_count() != graph.node_count() || rebuilt.edge_count() != cases.len() {
                return Err("population benchmark rebuild has the wrong graph size".into());
            }
        }
        let rebuild_time = rebuild_started.elapsed();

        let batched_rebuild_started = Instant::now();
        for _ in 0..rebuild_rounds {
            let mut rebuilt = Self::from_unknot(terminal_audit)?;
            for case in &cases {
                let outcome = rebuilt.relax_unknot_edge_impl(
                    case.source.clone(),
                    case.source_stop,
                    case.target_key,
                    case.program.clone(),
                    case.certificate_id,
                    case.certificate.clone(),
                    false,
                )?;
                if !matches!(outcome, RelaxationOutcome::Inserted { .. }) {
                    return Err("batched population relaxation did not insert its source".into());
                }
            }
            rebuilt.recompute_acs10_routes()?;
            if rebuilt.node_count() != graph.node_count() || rebuilt.edge_count() != cases.len() {
                return Err("batched population rebuild has the wrong graph size".into());
            }
        }
        let batched_rebuild_time = batched_rebuild_started.elapsed();

        Ok(PopulationUpdateBenchmark {
            load_time,
            active_unknot_edges: cases.len(),
            duplicate_relaxations: cases
                .len()
                .checked_mul(duplicate_rounds)
                .ok_or("duplicate relaxation count overflow")?,
            duplicate_time,
            refresh_rounds,
            refresh_time,
            rebuild_rounds,
            rebuild_relaxations: cases
                .len()
                .checked_mul(rebuild_rounds)
                .ok_or("rebuild relaxation count overflow")?,
            rebuild_time,
            batched_rebuild_relaxations: cases
                .len()
                .checked_mul(rebuild_rounds)
                .ok_or("batched rebuild relaxation count overflow")?,
            batched_rebuild_time,
        })
    }

    pub fn unknot_key(&self) -> Result<RepKey> {
        self.nodes
            .iter()
            .find_map(|(key, node)| {
                (node.representation.representation.strands == 1
                    && node.representation.representation.word.is_empty())
                .then_some(*key)
            })
            .ok_or_else(|| "population graph has no canonical unknot root".into())
    }

    pub fn u_upper_bound(&self, key: &RepKey) -> Option<u32> {
        self.nodes.get(key).map(|node| node.u_upper_bound)
    }

    pub fn max_route_rank(&self) -> Option<u64> {
        self.nodes.values().map(|node| node.route_rank).max()
    }

    pub fn seeds_with_route_rank(&self, route_rank: u64) -> Vec<PopulationSeed> {
        let mut seeds: Vec<_> = self
            .nodes
            .values()
            .filter(|node| node.route_rank == route_rank)
            .map(|node| PopulationSeed {
                representation: node.representation.clone(),
                u_upper_bound: node.u_upper_bound,
                route_rank: node.route_rank,
            })
            .collect();
        seeds.sort_by_key(|seed| seed.representation.key);
        seeds
    }

    pub fn seed(&self, key: &RepKey) -> Option<PopulationSeed> {
        self.nodes.get(key).map(|node| PopulationSeed {
            representation: node.representation.clone(),
            u_upper_bound: node.u_upper_bound,
            route_rank: node.route_rank,
        })
    }

    /// Return the complete optimization seed for one canonical graph key.
    /// Unlike the population-oriented ranking helpers, this is an exact lookup
    /// and preserves the source policy-stop attestation needed by a new edge.
    pub fn expansion_seed(&self, key: &RepKey) -> Result<Option<ExpansionSeed>> {
        let Some(node) = self.nodes.get(key) else {
            return Ok(None);
        };
        let active_witness_semantic_len = node
            .active_unknot_edge
            .map(|index| -> Result<u32> {
                let program = CheckpointedProofProgram::decode(
                    &self
                        .edges
                        .get(index)
                        .ok_or("active edge is absent")?
                        .program,
                )?;
                program
                    .instructions
                    .iter()
                    .filter(|instruction| matches!(instruction, ProofInstruction::Action(_)))
                    .count()
                    .try_into()
                    .map_err(|_| "proof semantic length exceeds u32".into())
            })
            .transpose()?
            .unwrap_or(0);
        Ok(Some(ExpansionSeed {
            representation: node.representation.clone(),
            policy_stop: node.policy_stop,
            u_upper_bound: node.u_upper_bound,
            route_rank: node.route_rank,
            active_witness_semantic_len,
        }))
    }

    /// Stable key/U materialization used to audit indirect Bellman changes.
    pub fn u_bounds(&self) -> Vec<(RepKey, u32)> {
        let mut bounds: Vec<_> = self
            .nodes
            .iter()
            .map(|(key, node)| (*key, node.u_upper_bound))
            .collect();
        bounds.sort_unstable_by_key(|(key, _)| *key);
        bounds
    }

    pub fn ranked_quality_seeds(&self, limit: usize) -> Result<Vec<ExpansionSeed>> {
        let mut seeds = self.expansion_seeds()?;
        seeds.sort_by_key(|seed| {
            (
                std::cmp::Reverse(seed.u_upper_bound),
                std::cmp::Reverse(seed.active_witness_semantic_len),
                seed.representation.representation.strands,
                seed.representation.representation.word.len(),
                seed.representation.key,
            )
        });
        seeds.truncate(limit);
        Ok(seeds)
    }

    pub fn ranked_short_state_seeds(&self, limit: usize) -> Result<Vec<ExpansionSeed>> {
        self.ranked_short_state_seeds_with_min_strands(limit, 1)
    }

    pub fn ranked_short_state_seeds_with_min_strands(
        &self,
        limit: usize,
        min_strands: u16,
    ) -> Result<Vec<ExpansionSeed>> {
        let mut seeds = self.expansion_seeds()?;
        seeds.retain(|seed| seed.representation.representation.strands >= min_strands);
        seeds.sort_by_key(|seed| {
            (
                seed.u_upper_bound,
                seed.representation.representation.strands,
                seed.representation.representation.word.len(),
                seed.active_witness_semantic_len,
                seed.representation.key,
            )
        });
        seeds.truncate(limit);
        Ok(seeds)
    }

    fn expansion_seeds(&self) -> Result<Vec<ExpansionSeed>> {
        self.nodes
            .values()
            .map(|node| {
                let active_witness_semantic_len = node
                    .active_unknot_edge
                    .map(|index| -> Result<u32> {
                        let program = CheckpointedProofProgram::decode(
                            &self
                                .edges
                                .get(index)
                                .ok_or("active edge is absent")?
                                .program,
                        )?;
                        program
                            .instructions
                            .iter()
                            .filter(|instruction| {
                                matches!(instruction, ProofInstruction::Action(_))
                            })
                            .count()
                            .try_into()
                            .map_err(|_| "proof semantic length exceeds u32".into())
                    })
                    .transpose()?
                    .unwrap_or(0);
                Ok(ExpansionSeed {
                    representation: node.representation.clone(),
                    policy_stop: node.policy_stop,
                    u_upper_bound: node.u_upper_bound,
                    route_rank: node.route_rank,
                    active_witness_semantic_len,
                })
            })
            .collect()
    }

    pub fn relax_unknot_edge(
        &mut self,
        source: crate::representation::NormalizedRepresentation,
        source_stop: PolicyStopAttestation,
        target_key: RepKey,
        program: CheckpointedProofProgram,
        certificate_id: Option<RepKey>,
    ) -> Result<RelaxationOutcome> {
        self.relax_unknot_edge_impl(
            source,
            source_stop,
            target_key,
            program,
            certificate_id,
            None,
            true,
        )
    }

    /// Validate and apply one relaxation while deferring only the materialized
    /// ACS10 route overlay. Call `recompute_acs10_routes` before reading or
    /// publishing ACS10 pointers.
    pub fn relax_unknot_edge_deferred_acs10(
        &mut self,
        source: crate::representation::NormalizedRepresentation,
        source_stop: PolicyStopAttestation,
        target_key: RepKey,
        program: CheckpointedProofProgram,
        certificate_id: Option<RepKey>,
    ) -> Result<RelaxationOutcome> {
        self.relax_unknot_edge_impl(
            source,
            source_stop,
            target_key,
            program,
            certificate_id,
            None,
            false,
        )
    }

    pub fn relax_planar_certificate_edge(
        &mut self,
        source: crate::representation::NormalizedRepresentation,
        source_stop: PolicyStopAttestation,
        target_key: RepKey,
        certificate: Vec<u8>,
    ) -> Result<RelaxationOutcome> {
        self.relax_unknot_edge_with_planar_certificate(
            source,
            source_stop,
            target_key,
            CheckpointedProofProgram {
                instructions: vec![ProofInstruction::Action(
                    SemanticAction::PlanarCertificateCollapse,
                )],
            },
            certificate,
            true,
        )
    }

    pub fn relax_unknot_edge_with_planar_certificate(
        &mut self,
        source: crate::representation::NormalizedRepresentation,
        source_stop: PolicyStopAttestation,
        target_key: RepKey,
        program: CheckpointedProofProgram,
        certificate: Vec<u8>,
        refresh_acs10: bool,
    ) -> Result<RelaxationOutcome> {
        if !program.contains_planar_certificate() {
            return Err("planar sidecar program has no collapse instruction".into());
        }
        let certificate_id = unknotdb::util::sha256(&certificate);
        self.relax_unknot_edge_impl(
            source,
            source_stop,
            target_key,
            program,
            Some(certificate_id),
            Some(certificate),
            refresh_acs10,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn relax_unknot_edge_impl(
        &mut self,
        source: crate::representation::NormalizedRepresentation,
        source_stop: PolicyStopAttestation,
        target_key: RepKey,
        program: CheckpointedProofProgram,
        certificate_id: Option<RepKey>,
        certificate: Option<Vec<u8>>,
        refresh_acs10: bool,
    ) -> Result<RelaxationOutcome> {
        if !source.representation.is_normalized()?
            || source.key != source.representation.normalize()?.key
        {
            return Err("population source is not mirror-orbit normalized".into());
        }
        if !source.representation.is_knot_closure()? {
            return Err("population source does not close to a knot".into());
        }
        validate_policy_stop(&source.representation, source_stop)?;
        let (target_encoding, target_u, target_route_rank, target_acs10) = {
            let target = self
                .nodes
                .get(&target_key)
                .ok_or("population edge target is not known")?;
            (
                target.representation.encoded.clone(),
                target.u_upper_bound,
                target.route_rank,
                complexity_acs10(&target.representation.representation, target.u_upper_bound)?,
            )
        };
        let program_bytes = program.encode()?;
        let cc_cost: u8 = program
            .cc_cost()
            .try_into()
            .map_err(|_| "population macro CC cost does not fit u8")?;
        if cc_cost > 1 {
            return Err("population macro must have zero or one crossing change".into());
        }
        let first_action = program.first_action()?.encode_u63()?;
        let proposed = target_u
            .checked_add(u32::from(cc_cost))
            .ok_or("population U upper bound overflow")?;
        let route_rank = target_route_rank
            .checked_add(1)
            .ok_or("population route rank overflow")?;
        let identity = edge_identity(
            source.key,
            target_key,
            cc_cost,
            first_action,
            &program_bytes,
        );
        let existing_edge_index = self.edge_lookup.get(&identity).and_then(|candidates| {
            candidates.iter().copied().find(|index| {
                self.edges
                    .get(*index)
                    .is_some_and(|edge| edge.program == program_bytes)
            })
        });

        let old = self.nodes.get(&source.key).map(|node| node.u_upper_bound);
        if let Some(current) = old {
            let source_acs10 = complexity_acs10(&source.representation, current)?;
            let useful_for_acs10 = target_acs10
                < source_acs10
                    .checked_add(u32::from(cc_cost))
                    .ok_or("ACS10 route comparison overflow")?
                && (target_acs10, target_key) < (source_acs10, source.key);
            if proposed >= current && !useful_for_acs10 {
                return Ok(RelaxationOutcome::Unchanged { current, proposed });
            }
            if proposed >= current && existing_edge_index.is_some() {
                return Ok(RelaxationOutcome::Unchanged { current, proposed });
            }
        }
        verify_edge_program_with_certificate(
            &source.encoded,
            &target_encoding,
            &program_bytes,
            CheckpointedProofProgram::VERSION,
            first_action,
            cc_cost,
            certificate.as_deref(),
        )?;
        let edge_index = if let Some(edge_index) = existing_edge_index {
            edge_index
        } else {
            let edge_index = self.edges.len();
            self.edges.push(KnowledgeEdge {
                source_key: source.key,
                target_key,
                cc_cost,
                first_action,
                program: program_bytes,
                certificate_id,
                certificate,
            });
            self.edge_lookup
                .entry(identity)
                .or_default()
                .push(edge_index);
            edge_index
        };
        let outcome = match self.nodes.get_mut(&source.key) {
            Some(node) => {
                let previous = node.u_upper_bound;
                if proposed < previous {
                    node.representation = source;
                    node.policy_stop = source_stop;
                    node.u_upper_bound = proposed;
                    node.route_rank = route_rank;
                    node.active_unknot_edge = Some(edge_index);
                    RelaxationOutcome::Improved {
                        old: previous,
                        new: proposed,
                    }
                } else {
                    RelaxationOutcome::AcceptedAcs10 {
                        current: previous,
                        proposed,
                    }
                }
            }
            None => {
                self.nodes.insert(
                    source.key,
                    KnowledgeNode {
                        representation: source,
                        policy_stop: source_stop,
                        u_upper_bound: proposed,
                        route_rank,
                        active_unknot_edge: Some(edge_index),
                        acs_route_rank: 0,
                        active_acs10_edge: None,
                    },
                );
                RelaxationOutcome::Inserted {
                    u_upper_bound: proposed,
                }
            }
        };
        if refresh_acs10 {
            self.recompute_acs10_routes()?;
        }
        Ok(outcome)
    }

    /// Rebuild the second routing layer from every stored proof edge.  The
    /// fixed `(ACS10, rep_key)` order makes equal-ACS, one-CC choices acyclic.
    pub fn recompute_acs10_routes(&mut self) -> Result<Acs10Refresh> {
        self.recompute_unknot_routes()?;
        self.recompute_acs10_routes_from_current_u()
    }

    fn recompute_acs10_routes_from_current_u(&mut self) -> Result<Acs10Refresh> {
        let acs10: HashMap<RepKey, u32> = self
            .nodes
            .iter()
            .map(|(key, node)| {
                Ok((
                    *key,
                    complexity_acs10(&node.representation.representation, node.u_upper_bound)?,
                ))
            })
            .collect::<Result<_>>()?;
        let mut order: Vec<_> = acs10.iter().map(|(key, value)| (*value, *key)).collect();
        order.sort_unstable();
        let ranks: HashMap<RepKey, u64> = order
            .into_iter()
            .enumerate()
            .map(|(rank, (_, key))| {
                Ok((
                    key,
                    rank.try_into().map_err(|_| "too many ACS10-ranked nodes")?,
                ))
            })
            .collect::<Result<_>>()?;
        for (key, node) in &mut self.nodes {
            node.acs_route_rank = ranks[key];
        }

        let mut refresh = Acs10Refresh::default();
        let mut best_by_source = HashMap::<RepKey, (u64, u32, u64, RepKey, usize)>::new();
        for (edge_index, edge) in self.edges.iter().enumerate() {
            let source_acs10 = acs10[&edge.source_key];
            let target_acs10 = acs10[&edge.target_key];
            let source_rank = ranks[&edge.source_key];
            let target_rank = ranks[&edge.target_key];
            if target_acs10
                >= source_acs10
                    .checked_add(u32::from(edge.cc_cost))
                    .ok_or("ACS10 route comparison overflow")?
                || target_rank >= source_rank
            {
                continue;
            }
            refresh.eligible_edges += 1;
            let candidate = (
                u64::from(target_acs10) + u64::from(edge.cc_cost),
                target_acs10,
                target_rank,
                edge.target_key,
                edge_index,
            );
            best_by_source
                .entry(edge.source_key)
                .and_modify(|current| {
                    if candidate < *current {
                        *current = candidate;
                    }
                })
                .or_insert(candidate);
        }
        let keys: Vec<_> = self.nodes.keys().copied().collect();
        for source_key in keys {
            let selected = best_by_source.get(&source_key).map(|candidate| candidate.4);
            let node = self
                .nodes
                .get_mut(&source_key)
                .ok_or("ACS10 source disappeared during refresh")?;
            if node.active_acs10_edge != selected {
                refresh.changed_routes += 1;
                node.active_acs10_edge = selected;
            }
            refresh.active_routes += usize::from(selected.is_some());
        }
        Ok(refresh)
    }

    /// Propagate only strict U improvements through reverse adjacency. This is
    /// the low-latency insertion path; large batches still use the independent
    /// full 0-1 BFS before publication. Equal-distance ties are deliberately
    /// left to the full deterministic refresh, so an incremental update cannot
    /// introduce a new zero-cost pointer cycle.
    pub fn relax_unknot_incremental(&mut self, seed_keys: &[RepKey]) -> Result<UnknotRefresh> {
        let before: HashMap<_, _> = self
            .nodes
            .iter()
            .map(|(key, node)| (*key, (node.u_upper_bound, node.active_unknot_edge)))
            .collect();
        let mut incoming = HashMap::<RepKey, Vec<usize>>::new();
        let mut outgoing = HashMap::<RepKey, Vec<usize>>::new();
        for (index, edge) in self.edges.iter().enumerate() {
            incoming.entry(edge.target_key).or_default().push(index);
            outgoing.entry(edge.source_key).or_default().push(index);
        }
        for indices in incoming.values_mut().chain(outgoing.values_mut()) {
            indices.sort_unstable();
        }
        let mut queue = VecDeque::new();
        let mut queued = HashSet::new();
        for &key in seed_keys {
            let Some(node) = self.nodes.get(&key) else {
                return Err("incremental U seed is not a graph node".into());
            };
            let current = node.u_upper_bound;
            let best = outgoing
                .get(&key)
                .into_iter()
                .flatten()
                .filter_map(|index| {
                    let edge = &self.edges[*index];
                    let target = self.nodes.get(&edge.target_key)?;
                    Some((target.u_upper_bound + u32::from(edge.cc_cost), *index))
                })
                .min_by_key(|(distance, index)| {
                    let edge = &self.edges[*index];
                    (*distance, edge.target_key, *index)
                });
            if let Some((distance, edge_index)) = best.filter(|(distance, _)| *distance < current) {
                let target = self.edges[edge_index].target_key;
                let rank = self.nodes[&target]
                    .route_rank
                    .checked_add(1)
                    .ok_or("incremental U rank overflow")?;
                let node = self.nodes.get_mut(&key).unwrap();
                node.u_upper_bound = distance;
                node.active_unknot_edge = Some(edge_index);
                node.route_rank = rank;
                queue.push_back(key);
                queued.insert(key);
            }
        }
        while let Some(target_key) = queue.pop_front() {
            queued.remove(&target_key);
            let target_u = self.nodes[&target_key].u_upper_bound;
            let target_rank = self.nodes[&target_key].route_rank;
            for &edge_index in incoming.get(&target_key).into_iter().flatten() {
                let edge = &self.edges[edge_index];
                let proposed = target_u
                    .checked_add(u32::from(edge.cc_cost))
                    .ok_or("incremental U overflow")?;
                let source = self.nodes.get_mut(&edge.source_key).unwrap();
                if proposed < source.u_upper_bound {
                    source.u_upper_bound = proposed;
                    source.active_unknot_edge = Some(edge_index);
                    source.route_rank = target_rank
                        .checked_add(1)
                        .ok_or("incremental U rank overflow")?;
                    if queued.insert(edge.source_key) {
                        queue.push_back(edge.source_key);
                    }
                }
            }
        }
        self.recompute_acs10_routes_from_current_u()?;
        let mut refresh = UnknotRefresh {
            reachable_nodes: self.nodes.len(),
            ..UnknotRefresh::default()
        };
        for (key, node) in &self.nodes {
            let (old_u, old_edge) = before[key];
            refresh.changed_bounds += usize::from(old_u != node.u_upper_bound);
            refresh.changed_routes += usize::from(old_edge != node.active_unknot_edge);
        }
        Ok(refresh)
    }

    /// Recompute U independently from all immutable proof edges. Distances use
    /// 0-1 BFS on reverse adjacency. A second deterministic tight-edge pass
    /// assigns pointers only toward already-ranked targets, preserving the
    /// historical lexicographic tie rule and cycle-safe zero-cost ranks.
    pub fn recompute_unknot_routes(&mut self) -> Result<UnknotRefresh> {
        let root = self.unknot_key()?;
        let mut incoming = HashMap::<RepKey, Vec<usize>>::new();
        for (index, edge) in self.edges.iter().enumerate() {
            incoming.entry(edge.target_key).or_default().push(index);
        }
        for edges in incoming.values_mut() {
            edges.sort_unstable_by_key(|index| {
                let edge = &self.edges[*index];
                (edge.source_key, edge.cc_cost, edge.first_action, *index)
            });
        }

        let mut distances = HashMap::<RepKey, u32>::from([(root, 0)]);
        let mut deque = VecDeque::from([root]);
        while let Some(target_key) = deque.pop_front() {
            let distance = distances[&target_key];
            for &edge_index in incoming.get(&target_key).into_iter().flatten() {
                let edge = &self.edges[edge_index];
                let proposed = distance
                    .checked_add(u32::from(edge.cc_cost))
                    .ok_or("U distance overflow")?;
                if distances
                    .get(&edge.source_key)
                    .is_none_or(|current| proposed < *current)
                {
                    distances.insert(edge.source_key, proposed);
                    if edge.cc_cost == 0 {
                        deque.push_front(edge.source_key);
                    } else {
                        deque.push_back(edge.source_key);
                    }
                }
            }
        }
        if distances.len() != self.nodes.len() {
            return Err(format!(
                "verified graph has {} nodes unreachable from the unknot",
                self.nodes.len() - distances.len()
            )
            .into());
        }

        let mut selected = HashMap::<RepKey, usize>::new();
        let mut ranked = HashSet::from([root]);
        let mut ranks = HashMap::<RepKey, u64>::from([(root, 0)]);
        let mut frontier = BinaryHeap::<Reverse<(u32, RepKey, RepKey, usize)>>::new();
        let push_tight_incoming = |target_key: RepKey,
                                   frontier: &mut BinaryHeap<
            Reverse<(u32, RepKey, RepKey, usize)>,
        >| {
            for &edge_index in incoming.get(&target_key).into_iter().flatten() {
                let edge = &self.edges[edge_index];
                if distances[&edge.source_key] == distances[&target_key] + u32::from(edge.cc_cost) {
                    frontier.push(Reverse((
                        distances[&edge.source_key],
                        edge.source_key,
                        target_key,
                        edge_index,
                    )));
                }
            }
        };
        push_tight_incoming(root, &mut frontier);
        while let Some(Reverse((_, source_key, _, edge_index))) = frontier.pop() {
            if !ranked.insert(source_key) {
                continue;
            }
            selected.insert(source_key, edge_index);
            let rank: u64 = ranks
                .len()
                .try_into()
                .map_err(|_| "too many unknot-ranked nodes")?;
            ranks.insert(source_key, rank);
            push_tight_incoming(source_key, &mut frontier);
        }
        if ranked.len() != self.nodes.len() {
            return Err("tight proof subgraph cannot assign cycle-safe unknot routes".into());
        }
        let mut refresh = UnknotRefresh {
            reachable_nodes: ranked.len(),
            ..UnknotRefresh::default()
        };
        for (key, node) in &mut self.nodes {
            let distance = distances[key];
            let edge = selected.get(key).copied();
            let rank = ranks[key];
            refresh.changed_bounds += usize::from(node.u_upper_bound != distance);
            refresh.changed_routes +=
                usize::from(node.active_unknot_edge != edge || node.route_rank != rank);
            node.u_upper_bound = distance;
            node.active_unknot_edge = edge;
            node.route_rank = rank;
        }
        Ok(refresh)
    }

    pub fn into_snapshot_records(mut self) -> Result<(Vec<NodeRecord>, Vec<EdgeRecord>)> {
        self.recompute_unknot_routes()?;
        self.recompute_acs10_routes()?;
        let mut keys: Vec<_> = self.nodes.keys().copied().collect();
        keys.sort_unstable();
        let ids: HashMap<RepKey, NodeId> = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let id: NodeId = index.try_into().map_err(|_| "too many population nodes")?;
                Ok((*key, id))
            })
            .collect::<Result<_>>()?;

        let edges: Vec<_> = self
            .edges
            .iter()
            .enumerate()
            .map(|(index, edge)| -> Result<EdgeRecord> {
                Ok(EdgeRecord {
                    edge_id: index.try_into().map_err(|_| "too many population edges")?,
                    source_node: ids[&edge.source_key],
                    target_node: ids[&edge.target_key],
                    cc_cost: edge.cc_cost,
                    first_action: edge.first_action,
                    program_version: CheckpointedProofProgram::VERSION,
                    program: edge.program.clone(),
                    certificate_id: edge.certificate_id,
                    certificate: edge.certificate.clone(),
                    validator_version: VALIDATOR_VERSION.into(),
                })
            })
            .collect::<Result<_>>()?;

        let nodes = keys
            .into_iter()
            .map(|key| -> Result<NodeRecord> {
                let node = &self.nodes[&key];
                let node_id = ids[&key];
                let active = node.active_unknot_edge.map(|edge_index| {
                    let edge = &self.edges[edge_index];
                    RouteStep {
                        edge_id: edge_index as u32,
                        target_node: ids[&edge.target_key],
                        cc_cost: edge.cc_cost,
                        first_action: edge.first_action,
                    }
                });
                let acs10 =
                    complexity_acs10(&node.representation.representation, node.u_upper_bound)?;
                let acs_route = node.active_acs10_edge.map(|edge_index| {
                    let edge = &self.edges[edge_index];
                    RouteStep {
                        edge_id: edge_index as u32,
                        target_node: ids[&edge.target_key],
                        cc_cost: edge.cc_cost,
                        first_action: edge.first_action,
                    }
                });
                Ok(NodeRecord {
                    rep_key: key,
                    node_id,
                    role_mask: ROLE_CORE,
                    encoding: node.representation.encoded.clone(),
                    u_upper_bound: Some(node.u_upper_bound),
                    unknot_route_rank: Some(node.route_rank),
                    acs10: Some(acs10),
                    acs_route_rank: Some(node.acs_route_rank),
                    next_unknot: active,
                    next_acs10: acs_route,
                    policy_stop: Some(node.policy_stop),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((nodes, edges))
    }
}

fn edge_identity(
    source_key: RepKey,
    target_key: RepKey,
    cc_cost: u8,
    first_action: u64,
    program: &[u8],
) -> EdgeIdentity {
    let mut hasher = DefaultHasher::new();
    program.hash(&mut hasher);
    EdgeIdentity {
        source_key,
        target_key,
        cc_cost,
        first_action,
        program_hash: hasher.finish(),
    }
}

fn build_edge_lookup(edges: &[KnowledgeEdge]) -> HashMap<EdgeIdentity, Vec<usize>> {
    let mut lookup = HashMap::with_capacity(edges.len());
    for (index, edge) in edges.iter().enumerate() {
        lookup
            .entry(edge_identity(
                edge.source_key,
                edge.target_key,
                edge.cc_cost,
                edge.first_action,
                &edge.program,
            ))
            .or_insert_with(Vec::new)
            .push(index);
    }
    lookup
}

fn key_from_blob(blob: &[u8], label: &str) -> Result<RepKey> {
    blob.try_into()
        .map_err(|_| format!("{} must contain exactly 32 bytes", label).into())
}

fn nonnegative_u64(value: i64, label: &str) -> Result<u64> {
    value
        .try_into()
        .map_err(|_| format!("{} is negative", label).into())
}

fn nonnegative_usize(value: i64, label: &str) -> Result<usize> {
    value
        .try_into()
        .map_err(|_| format!("{} is negative or too large", label).into())
}

pub fn complexity_acs10(representation: &BraidRepresentation, u_upper_bound: u32) -> Result<u32> {
    let value = 10_u64
        .checked_mul(u64::from(representation.strands))
        .and_then(|value| value.checked_add(5 * u64::from(u_upper_bound)))
        .and_then(|value| value.checked_add(representation.word.len() as u64))
        .ok_or("ACS10 overflow")?;
    value
        .try_into()
        .map_err(|_| "ACS10 does not fit u32".into())
}

fn validate_policy_stop(
    representation: &BraidRepresentation,
    stop: PolicyStopAttestation,
) -> Result<()> {
    match stop {
        PolicyStopAttestation::PreferredCrossingChange { action, .. } => {
            let action = SemanticAction::decode_u63(action)?;
            if !action.is_crossing_change() {
                return Err("population policy stop does not prefer CC".into());
            }
            action.apply(representation)?;
        }
        PolicyStopAttestation::Terminal { .. } => {
            if representation.strands != 1 || !representation.word.is_empty() {
                return Err("population terminal stop is not B1 []".into());
            }
        }
        PolicyStopAttestation::CapacityFallback { .. } => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::representation::ProofInstruction;

    fn trefoil_fixture() -> (
        crate::representation::NormalizedRepresentation,
        PolicyStopAttestation,
        CheckpointedProofProgram,
    ) {
        let trefoil = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        }
        .normalize()
        .unwrap();
        let stop = PolicyStopAttestation::PreferredCrossingChange {
            action: SemanticAction::CrossingChange { position: 0 }
                .encode_u63()
                .unwrap(),
            audit_sha256: [2; 32],
        };
        let program = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::Action(SemanticAction::Insert {
                    position: 0,
                    generator: 1,
                    sign: 1,
                }),
                ProofInstruction::Action(SemanticAction::Reduce { position: 0 }),
                ProofInstruction::Action(SemanticAction::CrossingChange { position: 0 }),
                ProofInstruction::Action(SemanticAction::Reduce { position: 0 }),
                ProofInstruction::Action(SemanticAction::Destabilize),
            ],
        };
        (trefoil, stop, program)
    }

    #[test]
    fn bellman_relaxation_inserts_deduplicates_and_improves() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let unknot_key = *graph.nodes.keys().next().unwrap();
        let (trefoil, stop, program) = trefoil_fixture();

        graph.nodes.get_mut(&unknot_key).unwrap().u_upper_bound = 5;
        assert_eq!(
            graph
                .relax_unknot_edge(trefoil.clone(), stop, unknot_key, program.clone(), None)
                .unwrap(),
            RelaxationOutcome::Inserted { u_upper_bound: 6 }
        );
        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.u_upper_bound(&unknot_key), Some(0));
        assert_eq!(graph.u_upper_bound(&trefoil.key), Some(1));
        assert!(matches!(
            graph
                .relax_unknot_edge(trefoil.clone(), stop, unknot_key, program.clone(), None)
                .unwrap(),
            RelaxationOutcome::Unchanged { .. }
        ));
        assert!(matches!(
            graph
                .relax_unknot_edge(trefoil.clone(), stop, unknot_key, program, None)
                .unwrap(),
            RelaxationOutcome::Unchanged { .. }
        ));
        assert_eq!(graph.u_upper_bound(&trefoil.key), Some(1));
        assert_eq!(graph.edge_count(), 1);
        let (nodes, edges) = graph.into_snapshot_records().unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);
        assert_eq!(
            nodes
                .iter()
                .find(|node| node.rep_key == trefoil.key)
                .unwrap()
                .u_upper_bound,
            Some(1)
        );
    }

    #[test]
    fn deferred_relaxation_materializes_acs10_only_on_refresh() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let unknot_key = graph.unknot_key().unwrap();
        let (trefoil, stop, program) = trefoil_fixture();
        assert!(matches!(
            graph
                .relax_unknot_edge_deferred_acs10(trefoil.clone(), stop, unknot_key, program, None,)
                .unwrap(),
            RelaxationOutcome::Inserted { .. }
        ));
        assert_eq!(graph.nodes[&trefoil.key].active_acs10_edge, None);
        let refresh = graph.recompute_acs10_routes().unwrap();
        assert_eq!(refresh.changed_routes, 1);
        assert_eq!(graph.nodes[&trefoil.key].active_acs10_edge, Some(0));
    }

    #[test]
    fn production_snapshot_round_trips_into_resumable_population() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let unknot_key = graph.unknot_key().unwrap();
        let (trefoil, stop, program) = trefoil_fixture();
        graph
            .relax_unknot_edge(trefoil.clone(), stop, unknot_key, program, None)
            .unwrap();
        let (nodes, edges) = graph.into_snapshot_records().unwrap();
        let path = std::env::temp_dir().join(format!(
            "unknotdb-population-resume-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let model_id = format!("resume-test:{}", "a".repeat(64));
        crate::write_snapshot_atomic(
            &path,
            &SnapshotMeta::braid_mirror_orbit_v1("round-trip-test", &model_id),
            nodes,
            edges,
        )
        .unwrap();

        let (resumed, meta) = PopulationGraph::from_snapshot(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(meta.policy_model_id.as_deref(), Some(model_id.as_str()));
        assert_eq!(resumed.node_count(), 2);
        assert_eq!(resumed.edge_count(), 1);
        assert_eq!(resumed.max_route_rank(), Some(1));
        assert_eq!(resumed.seed(&trefoil.key).unwrap().u_upper_bound, 1);
    }

    #[test]
    fn verified_acs10_candidate_is_kept_when_it_does_not_improve_u() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let unknot_key = graph.unknot_key().unwrap();
        let simple = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1],
        }
        .normalize()
        .unwrap();
        let simple_stop = PolicyStopAttestation::PreferredCrossingChange {
            action: SemanticAction::CrossingChange { position: 0 }
                .encode_u63()
                .unwrap(),
            audit_sha256: [3; 32],
        };
        graph
            .relax_unknot_edge(
                simple.clone(),
                simple_stop,
                unknot_key,
                CheckpointedProofProgram {
                    instructions: vec![
                        ProofInstruction::Action(SemanticAction::CrossingChange { position: 0 }),
                        ProofInstruction::Action(SemanticAction::Destabilize),
                    ],
                },
                None,
            )
            .unwrap();
        let (trefoil, trefoil_stop, direct_program) = trefoil_fixture();
        graph
            .relax_unknot_edge(
                trefoil.clone(),
                trefoil_stop,
                unknot_key,
                direct_program,
                None,
            )
            .unwrap();

        let outcome = graph
            .relax_unknot_edge(
                trefoil,
                trefoil_stop,
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
        assert_eq!(
            outcome,
            RelaxationOutcome::AcceptedAcs10 {
                current: 1,
                proposed: 2
            }
        );
        assert_eq!(graph.edge_count(), 3);
    }

    #[test]
    fn acs10_pointer_is_recomputed_independently_from_unknot_pointer() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let root_key = graph.unknot_key().unwrap();
        let u_target = BraidRepresentation {
            strands: 5,
            cyclic_band_generators: false,
            word: vec![1],
        }
        .normalize()
        .unwrap();
        let source = BraidRepresentation {
            strands: 6,
            cyclic_band_generators: false,
            word: vec![1],
        }
        .normalize()
        .unwrap();
        let stop = PolicyStopAttestation::PreferredCrossingChange {
            action: SemanticAction::CrossingChange { position: 0 }
                .encode_u63()
                .unwrap(),
            audit_sha256: [4; 32],
        };
        graph.nodes.insert(
            u_target.key,
            KnowledgeNode {
                representation: u_target.clone(),
                policy_stop: stop,
                u_upper_bound: 0,
                route_rank: 0,
                active_unknot_edge: None,
                acs_route_rank: 0,
                active_acs10_edge: None,
            },
        );
        graph.nodes.insert(
            source.key,
            KnowledgeNode {
                representation: source.clone(),
                policy_stop: stop,
                u_upper_bound: 5,
                route_rank: 1,
                active_unknot_edge: Some(0),
                acs_route_rank: 0,
                active_acs10_edge: None,
            },
        );
        let first_action = SemanticAction::CrossingChange { position: 0 }
            .encode_u63()
            .unwrap();
        graph.edges = vec![
            KnowledgeEdge {
                source_key: u_target.key,
                target_key: root_key,
                cc_cost: 0,
                first_action,
                program: vec![],
                certificate_id: None,
                certificate: None,
            },
            KnowledgeEdge {
                source_key: source.key,
                target_key: u_target.key,
                cc_cost: 0,
                first_action,
                program: vec![],
                certificate_id: None,
                certificate: None,
            },
            KnowledgeEdge {
                source_key: source.key,
                target_key: root_key,
                cc_cost: 1,
                first_action,
                program: vec![],
                certificate_id: None,
                certificate: None,
            },
        ];

        let refresh = graph.recompute_acs10_routes().unwrap();
        let source_node = &graph.nodes[&source.key];
        assert_eq!(source_node.active_unknot_edge, Some(1));
        assert_eq!(source_node.active_acs10_edge, Some(2));
        assert_eq!(refresh.active_routes, 2);
        assert_eq!(refresh.eligible_edges, 3);
    }

    #[test]
    fn incremental_strict_relaxation_agrees_with_full_zero_one_bfs() {
        let mut graph = PopulationGraph::from_unknot([1; 32]).unwrap();
        let root = graph.unknot_key().unwrap();
        let source = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        }
        .normalize()
        .unwrap();
        let stop = PolicyStopAttestation::PreferredCrossingChange {
            action: SemanticAction::CrossingChange { position: 1 }
                .encode_u63()
                .unwrap(),
            audit_sha256: [2; 32],
        };
        graph.nodes.insert(
            source.key,
            KnowledgeNode {
                representation: source.clone(),
                policy_stop: stop,
                u_upper_bound: 99,
                route_rank: 99,
                active_unknot_edge: None,
                acs_route_rank: 0,
                active_acs10_edge: None,
            },
        );
        graph.edges.push(KnowledgeEdge {
            source_key: source.key,
            target_key: root,
            cc_cost: 1,
            first_action: SemanticAction::CrossingChange { position: 1 }
                .encode_u63()
                .unwrap(),
            program: Vec::new(),
            certificate_id: None,
            certificate: None,
        });
        let mut full = graph.clone();
        graph.relax_unknot_incremental(&[source.key]).unwrap();
        full.recompute_unknot_routes().unwrap();
        assert_eq!(
            graph.nodes[&source.key].u_upper_bound,
            full.nodes[&source.key].u_upper_bound
        );
        assert_eq!(
            graph.nodes[&source.key].active_unknot_edge,
            full.nodes[&source.key].active_unknot_edge
        );
    }
}
