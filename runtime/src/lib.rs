//! Immutable SQLite snapshots and a compact in-memory lookup layer.
//!
//! This crate is deliberately separate from the dependency-free proof verifier.
//! A snapshot may only contain edges already accepted by that verifier.  This
//! crate checks storage integrity and routing invariants; it does not turn an
//! unverified program into a proof.

use arc_swap::ArcSwap;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, MAIN_DB};
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod campaign;
pub mod catalogue_braid_import;
pub mod catalogue_planar_import;
pub mod census;
pub mod descending;
pub mod expansion;
pub mod frontier;
pub mod high_u;
pub mod optimizer;
pub mod planar;
pub mod planar_import;
pub mod policy;
pub mod population;
pub mod reducer;
pub mod representation;
pub mod rf_import;
pub mod targeted;
pub mod wang_zhang_import;

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
pub type RepKey = [u8; 32];
pub type NodeId = u32;
pub type EdgeId = u32;

pub const SCHEMA_VERSION: u32 = 4;
pub const ROLE_CORE: u8 = 1;
pub const ROLE_L10_AUX: u8 = 2;
pub const ROLE_L1000_AUX: u8 = 4;

const HOT_LOOKUP_SQL: &str = "
SELECT n.node_id, n.u_upper_bound, n.acs10,
       next_unknot_edge, next_unknot_target, next_unknot_cc_cost,
       first_unknot_action,
       next_acs10_edge, next_acs10_target, next_acs10_cc_cost,
       first_acs10_action
FROM node_keys k JOIN nodes n ON n.node_id=k.node_id
WHERE k.rep_key = ?1
";

const LEGACY_HOT_LOOKUP_SQL: &str = "
SELECT node_id, u_upper_bound, acs10,
       next_unknot_edge, next_unknot_target, next_unknot_cc_cost,
       first_unknot_action,
       next_acs10_edge, next_acs10_target, next_acs10_cc_cost,
       first_acs10_action
FROM nodes WHERE rep_key = ?1
";

const LOAD_NODES_SQL: &str = "
SELECT node_id, u_upper_bound, acs10,
       next_unknot_edge, next_unknot_target, next_unknot_cc_cost,
       first_unknot_action,
       next_acs10_edge, next_acs10_target, next_acs10_cc_cost,
       first_acs10_action
FROM nodes ORDER BY node_id
";

const LEGACY_LOAD_NODES_SQL: &str = "
SELECT rep_key, node_id, u_upper_bound, acs10,
       next_unknot_edge, next_unknot_target, next_unknot_cc_cost,
       first_unknot_action,
       next_acs10_edge, next_acs10_target, next_acs10_cc_cost,
       first_acs10_action
FROM nodes ORDER BY node_id
";

const SCHEMA_SQL: &str = r#"
PRAGMA application_id = 1430996034;
PRAGMA user_version = 4;

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- This is the hot table. Large representations and proof programs live in
-- separate tables so an exact key lookup touches only a small B-tree row.
CREATE TABLE nodes (
    node_id                  INTEGER PRIMARY KEY CHECK(node_id >= 0),
    role_mask                INTEGER NOT NULL CHECK(role_mask BETWEEN 1 AND 7),
    u_upper_bound            INTEGER CHECK(u_upper_bound >= 0),
    unknot_route_rank        INTEGER CHECK(unknot_route_rank >= 0),
    acs10                    INTEGER CHECK(acs10 >= 0),
    acs_route_rank           INTEGER CHECK(acs_route_rank >= 0),

    next_unknot_edge         INTEGER,
    next_unknot_target       INTEGER,
    next_unknot_cc_cost      INTEGER CHECK(next_unknot_cc_cost BETWEEN 0 AND 1),
    first_unknot_action      INTEGER CHECK(first_unknot_action >= 0),

    next_acs10_edge          INTEGER,
    next_acs10_target        INTEGER,
    next_acs10_cc_cost       INTEGER CHECK(next_acs10_cc_cost BETWEEN 0 AND 1),
    first_acs10_action       INTEGER CHECK(first_acs10_action >= 0),

    FOREIGN KEY(next_unknot_edge) REFERENCES edges(edge_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(next_acs10_edge) REFERENCES edges(edge_id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE node_keys (
    rep_key BLOB PRIMARY KEY CHECK(length(rep_key) = 32),
    node_id INTEGER NOT NULL,
    -- 0: mirror/origin canonical key; 1: exact unnormalized checkpoint key.
    key_kind INTEGER NOT NULL CHECK(key_kind BETWEEN 0 AND 1),
    FOREIGN KEY(node_id) REFERENCES nodes(node_id)
) WITHOUT ROWID;

CREATE TABLE representations (
    node_id   INTEGER PRIMARY KEY,
    encoding  BLOB NOT NULL,
    FOREIGN KEY(node_id) REFERENCES nodes(node_id)
) WITHOUT ROWID;

-- Every production vertex is attested against exactly one snapshot-wide
-- policy model/objective/controller contract. The CC preference is heuristic
-- and reproducible from the frozen model; action legality remains exact here.
CREATE TABLE policy_stops (
    node_id                 INTEGER PRIMARY KEY,
    stop_kind               INTEGER NOT NULL CHECK(stop_kind BETWEEN 0 AND 2),
    preferred_cc_action     INTEGER CHECK(preferred_cc_action >= 0),
    audit_sha256            BLOB NOT NULL CHECK(length(audit_sha256) = 32),
    CHECK((stop_kind=0 AND preferred_cc_action IS NOT NULL) OR
          (stop_kind IN (1,2) AND preferred_cc_action IS NULL)),
    FOREIGN KEY(node_id) REFERENCES nodes(node_id)
) WITHOUT ROWID;

CREATE TABLE programs (
    program_id          INTEGER PRIMARY KEY CHECK(program_id >= 0),
    program_version     INTEGER NOT NULL CHECK(program_version >= 0),
    program             BLOB NOT NULL,
    program_sha256      BLOB NOT NULL CHECK(length(program_sha256) = 32)
);

-- Optional cold proof sidecars. The primary key is SHA-256(certificate).
-- Legacy certificate_id annotations need not have a row; the planar macro
-- requires one and is rejected without it.
CREATE TABLE planar_certificates (
    certificate_id      BLOB PRIMARY KEY CHECK(length(certificate_id) = 32),
    certificate_version INTEGER NOT NULL CHECK(certificate_version >= 0),
    certificate         BLOB NOT NULL
) WITHOUT ROWID;

CREATE TABLE edges (
    edge_id             INTEGER PRIMARY KEY CHECK(edge_id >= 0),
    source_node         INTEGER NOT NULL,
    target_node         INTEGER NOT NULL,
    cc_cost             INTEGER NOT NULL CHECK(cc_cost BETWEEN 0 AND 1),
    first_action        INTEGER NOT NULL CHECK(first_action >= 0),
    program_id          INTEGER NOT NULL,
    program_anchor_x    INTEGER NOT NULL CHECK(program_anchor_x BETWEEN 0 AND 65535),
    program_anchor_y    INTEGER NOT NULL CHECK(program_anchor_y BETWEEN 0 AND 4294967295),
    certificate_id      BLOB CHECK(certificate_id IS NULL OR length(certificate_id) = 32),
    FOREIGN KEY(program_id) REFERENCES programs(program_id),
    FOREIGN KEY(source_node) REFERENCES nodes(node_id),
    FOREIGN KEY(target_node) REFERENCES nodes(node_id)
);

CREATE INDEX edges_by_source ON edges(source_node);
CREATE INDEX edges_by_target ON edges(target_node);
CREATE INDEX edges_by_program ON edges(program_id);
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotMeta {
    pub snapshot_version: String,
    pub key_spec: String,
    pub normalizer_version: String,
    pub representation_codec: String,
    pub action_codec: String,
    pub validator_version: String,
    pub source_generation: String,
    pub policy_model_id: Option<String>,
    pub policy_objective_ratio: Option<u32>,
    pub policy_adapter_version: Option<String>,
    pub controller_initial_state: Option<String>,
    pub synthetic: bool,
}

impl SnapshotMeta {
    pub fn synthetic() -> Self {
        Self {
            snapshot_version: "synthetic-v0".into(),
            key_spec: "synthetic-unique-32-byte-v0".into(),
            normalizer_version: "synthetic-v0".into(),
            representation_codec: "synthetic-u64-v0".into(),
            action_codec: "synthetic-u63-v0".into(),
            validator_version: "synthetic-structural-only-v0".into(),
            source_generation: "benchmark".into(),
            policy_model_id: None,
            policy_objective_ratio: None,
            policy_adapter_version: None,
            controller_initial_state: None,
            synthetic: true,
        }
    }

    pub fn braid_mirror_orbit_v1(
        source_generation: impl Into<String>,
        policy_model_id: impl Into<String>,
    ) -> Self {
        Self {
            snapshot_version: "braid-mirror-orbit-graph-v1".into(),
            key_spec: representation::KEY_SPEC.into(),
            normalizer_version: representation::NORMALIZER_VERSION.into(),
            representation_codec: representation::PACKED_REPRESENTATION_CODEC.into(),
            action_codec: representation::ACTION_CODEC.into(),
            validator_version: representation::VALIDATOR_VERSION.into(),
            source_generation: source_generation.into(),
            policy_model_id: Some(policy_model_id.into()),
            policy_objective_ratio: Some(policy::GRAPH_STOP_OBJECTIVE_RATIO),
            policy_adapter_version: Some(policy::POLICY_ADAPTER_VERSION.into()),
            controller_initial_state: Some(policy::CONTROLLER_INITIAL_STATE.into()),
            synthetic: false,
        }
    }

    fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("snapshot_version", self.snapshot_version.as_str()),
            ("key_spec", self.key_spec.as_str()),
            ("normalizer_version", self.normalizer_version.as_str()),
            ("representation_codec", self.representation_codec.as_str()),
            ("action_codec", self.action_codec.as_str()),
            ("validator_version", self.validator_version.as_str()),
            ("source_generation", self.source_generation.as_str()),
        ] {
            if value.is_empty() {
                return Err(format!("metadata `{}` is empty", name).into());
            }
        }
        if !self.synthetic
            && (self.key_spec != representation::KEY_SPEC
                || self.normalizer_version != representation::NORMALIZER_VERSION
                || !matches!(
                    self.representation_codec.as_str(),
                    representation::REPRESENTATION_CODEC
                        | representation::PACKED_REPRESENTATION_CODEC
                )
                || self.action_codec != representation::ACTION_CODEC)
        {
            return Err(
                "non-synthetic snapshots must use supported frozen braid-cylinder codecs".into(),
            );
        }
        if self.synthetic {
            if self.policy_model_id.is_some()
                || self.policy_objective_ratio.is_some()
                || self.policy_adapter_version.is_some()
                || self.controller_initial_state.is_some()
            {
                return Err("synthetic snapshots cannot claim a policy stopping contract".into());
            }
        } else {
            if self.policy_objective_ratio != Some(policy::GRAPH_STOP_OBJECTIVE_RATIO)
                || !self
                    .policy_adapter_version
                    .as_deref()
                    .is_some_and(policy::is_supported_snapshot_adapter)
                || self.controller_initial_state.as_deref()
                    != Some(policy::CONTROLLER_INITIAL_STATE)
            {
                return Err(
                    "production snapshot has an incomplete policy stopping contract".into(),
                );
            }
            let model_id = self
                .policy_model_id
                .as_deref()
                .ok_or("production snapshot has no policy model ID")?;
            validate_policy_model_id(model_id)?;
        }
        Ok(())
    }
}

fn validate_policy_model_id(model_id: &str) -> Result<()> {
    let Some((lineage, sha256)) = model_id.rsplit_once(':') else {
        return Err("policy model id must end in a SHA-256 component".into());
    };
    if lineage.is_empty()
        || sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("policy model id must end in 64 lowercase SHA-256 hex digits".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteStep {
    pub edge_id: EdgeId,
    pub target_node: NodeId,
    pub cc_cost: u8,
    /// Compact, codec-versioned first primitive action. The top bit is reserved.
    pub first_action: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeRecord {
    pub rep_key: RepKey,
    pub node_id: NodeId,
    pub role_mask: u8,
    pub encoding: Vec<u8>,
    pub u_upper_bound: Option<u32>,
    /// Strictly decreases along the selected U route, including zero-CC edges.
    pub unknot_route_rank: Option<u64>,
    pub acs10: Option<u32>,
    /// A snapshot-local topological order for the selected ACS10 route.
    pub acs_route_rank: Option<u64>,
    pub next_unknot: Option<RouteStep>,
    pub next_acs10: Option<RouteStep>,
    pub policy_stop: Option<PolicyStopAttestation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyStopAttestation {
    PreferredCrossingChange {
        action: u64,
        audit_sha256: RepKey,
    },
    Terminal {
        audit_sha256: RepKey,
    },
    /// The pinned model could not encode this state. The stored audit proves
    /// deterministic decreasing reduction plus exact canonicalization; no
    /// neural-network action was asserted.
    CapacityFallback {
        audit_sha256: RepKey,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeRecord {
    pub edge_id: EdgeId,
    pub source_node: NodeId,
    pub target_node: NodeId,
    pub cc_cost: u8,
    pub first_action: u64,
    pub program_version: u32,
    pub program: Vec<u8>,
    pub certificate_id: Option<RepKey>,
    pub certificate: Option<Vec<u8>>,
    pub validator_version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotRoute {
    pub node_id: NodeId,
    pub u_upper_bound: Option<u32>,
    pub acs10: Option<u32>,
    pub next_unknot: Option<RouteStep>,
    pub next_acs10: Option<RouteStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeHot {
    pub edge_id: EdgeId,
    pub source_node: NodeId,
    pub target_node: NodeId,
    pub cc_cost: u8,
    pub first_action: u64,
    pub program_version: u32,
    /// Programs remain in SQLite's cold blob arena; the hot graph keeps only
    /// their size and routing header.
    pub program_len: u32,
}

#[derive(Debug)]
pub struct GraphSnapshot {
    meta: SnapshotMeta,
    keys: Vec<RepKey>,
    nodes: Vec<HotRoute>,
    edges: Vec<EdgeHot>,
    by_key: HashMap<RepKey, NodeId>,
}

impl GraphSnapshot {
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self> {
        let db = SqliteSnapshot::open_file(path)?;
        Self::load_connection(&db.conn)
    }

    pub fn load_deserialized(path: impl AsRef<Path>) -> Result<Self> {
        let db = SqliteSnapshot::open_deserialized(path)?;
        Self::load_connection(&db.conn)
    }

    pub fn load_connection(conn: &Connection) -> Result<Self> {
        validate_connection(conn)?;
        let meta = read_meta(conn)?;

        let node_count = scalar_usize(conn, "SELECT count(*) FROM nodes")?;
        let edge_count = scalar_usize(conn, "SELECT count(*) FROM edges")?;
        let mut keys = Vec::with_capacity(node_count);
        let mut nodes = Vec::with_capacity(node_count);
        let mut by_key = HashMap::with_capacity(node_count);
        let schema = snapshot_schema_version(conn)?;
        if schema >= 2 {
            let mut stmt = conn.prepare(LOAD_NODES_SQL)?;
            let rows = stmt.query_map([], |row| hot_route_from_row(row, 0))?;
            for row in rows {
                let hot = row?;
                let expected = nodes.len() as NodeId;
                if hot.node_id != expected {
                    return Err(format!(
                        "node ids are not dense: expected {}, found {}",
                        expected, hot.node_id
                    )
                    .into());
                }
                nodes.push(hot);
            }
            keys.resize(node_count, [0; 32]);
            let mut seen = vec![false; node_count];
            let mut stmt = conn.prepare("SELECT rep_key, node_id FROM node_keys")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (raw_key, raw_id) = row?;
                let key = rep_key_from_slice(&raw_key)?;
                let id = u32_from_i64(raw_id, "node_keys.node_id")?;
                let slot = seen
                    .get_mut(id as usize)
                    .ok_or("node key points outside the dense node arena")?;
                if std::mem::replace(slot, true) {
                    return Err(format!("node {} has more than one key", id).into());
                }
                keys[id as usize] = key;
                if by_key.insert(key, id).is_some() {
                    return Err("duplicate rep_key".into());
                }
            }
            if seen.iter().any(|value| !value) {
                return Err("a dense node has no rep_key".into());
            }
        } else {
            let mut stmt = conn.prepare(LEGACY_LOAD_NODES_SQL)?;
            let rows = stmt.query_map([], |row| {
                let key: Vec<u8> = row.get(0)?;
                let hot = hot_route_from_row(row, 1)?;
                Ok((key, hot))
            })?;
            for row in rows {
                let (raw_key, hot) = row?;
                let key = rep_key_from_slice(&raw_key)?;
                let expected = nodes.len() as NodeId;
                if hot.node_id != expected {
                    return Err(format!(
                        "node ids are not dense: expected {}, found {}",
                        expected, hot.node_id
                    )
                    .into());
                }
                if by_key.insert(key, hot.node_id).is_some() {
                    return Err("duplicate rep_key".into());
                }
                keys.push(key);
                nodes.push(hot);
            }
        }

        let mut edges = Vec::with_capacity(edge_count);
        let edge_sql = if schema == 0 {
            "SELECT edge_id, source_node, target_node, cc_cost, first_action, \
                    program_version, length(program) \
             FROM edges ORDER BY edge_id"
        } else {
            "SELECT e.edge_id, e.source_node, e.target_node, e.cc_cost, e.first_action, \
                    p.program_version, length(p.program) \
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
                row.get::<_, i64>(6)?,
            ))
        })?;
        for row in rows {
            let (id, source, target, cost, action, version, program_len) = row?;
            let expected = edges.len() as u32;
            if u32_from_i64(id, "edge_id")? != expected {
                return Err(format!(
                    "edge ids are not dense: expected {}, found {}",
                    expected, id
                )
                .into());
            }
            edges.push(EdgeHot {
                edge_id: expected,
                source_node: u32_from_i64(source, "source_node")?,
                target_node: u32_from_i64(target, "target_node")?,
                cc_cost: u8_from_i64(cost, "cc_cost")?,
                first_action: u64_from_i64(action, "first_action")?,
                program_version: u32_from_i64(version, "program_version")?,
                program_len: u32_from_i64(program_len, "program_len")?,
            });
        }

        Ok(Self {
            meta,
            keys,
            nodes,
            edges,
            by_key,
        })
    }

    #[inline]
    pub fn lookup(&self, key: &RepKey) -> Option<HotRoute> {
        let id = *self.by_key.get(key)? as usize;
        self.nodes.get(id).copied()
    }

    pub fn edge(&self, id: EdgeId) -> Option<&EdgeHot> {
        self.edges.get(id as usize)
    }

    pub fn key_for_node(&self, id: NodeId) -> Option<&RepKey> {
        self.keys.get(id as usize)
    }

    pub fn meta(&self) -> &SnapshotMeta {
        &self.meta
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// Readers load an `Arc` without a lock. A rebuilt, fully checked snapshot is
/// published in one atomic swap; existing requests finish on the old snapshot.
pub struct LiveGraph {
    current: ArcSwap<GraphSnapshot>,
}

impl LiveGraph {
    pub fn new(snapshot: GraphSnapshot) -> Self {
        Self {
            current: ArcSwap::from_pointee(snapshot),
        }
    }

    pub fn load(&self) -> Arc<GraphSnapshot> {
        self.current.load_full()
    }

    pub fn publish(&self, snapshot: GraphSnapshot) -> Arc<GraphSnapshot> {
        self.current.swap(Arc::new(snapshot))
    }
}

pub struct SqliteSnapshot {
    conn: Connection,
    schema_version: u32,
}

impl SqliteSnapshot {
    pub fn open_file(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )?;
        conn.pragma_update(None, "query_only", true)?;
        conn.pragma_update(None, "mmap_size", 1_i64 << 30)?;
        validate_header(&conn)?;
        let schema_version = snapshot_schema_version(&conn)?;
        Ok(Self {
            conn,
            schema_version,
        })
    }

    pub fn open_deserialized(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        let size: usize = file
            .metadata()?
            .len()
            .try_into()
            .map_err(|_| "snapshot does not fit in address space")?;
        let mut conn = Connection::open_in_memory()?;
        conn.deserialize_read_exact(MAIN_DB, file, size, true)?;
        conn.pragma_update(None, "query_only", true)?;
        validate_header(&conn)?;
        let schema_version = snapshot_schema_version(&conn)?;
        Ok(Self {
            conn,
            schema_version,
        })
    }

    #[inline]
    pub fn lookup(&self, key: &RepKey) -> Result<Option<HotRoute>> {
        let sql = if self.schema_version >= 2 {
            HOT_LOOKUP_SQL
        } else {
            LEGACY_HOT_LOOKUP_SQL
        };
        let mut stmt = self.conn.prepare_cached(sql)?;
        let hit = stmt
            .query_row(params![key.as_slice()], |row| hot_route_from_row(row, 0))
            .optional()?;
        Ok(hit)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.schema_version)
    }

    /// Full cold-arena integrity pass. Production publication should run this
    /// once before making a snapshot visible; hot readers need not re-hash
    /// every proof program on each process start.
    pub fn verify_program_hashes(&self) -> Result<()> {
        if snapshot_schema_version(&self.conn)? == 0 {
            let mut stmt = self
                .conn
                .prepare("SELECT edge_id, program, program_sha256 FROM edges ORDER BY edge_id")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (edge_id, program, stated) = row?;
                if stated != unknotdb::util::sha256_hex(&program) {
                    return Err(format!("edge {} program hash mismatch", edge_id).into());
                }
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT program_id, program, program_sha256 FROM programs ORDER BY program_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })?;
            for row in rows {
                let (program_id, program, stated) = row?;
                if stated.as_slice() != unknotdb::util::sha256(&program) {
                    return Err(format!("program {} hash mismatch", program_id).into());
                }
            }
        }
        if table_exists(&self.conn, "planar_certificates")? {
            let mut stmt = self.conn.prepare(
                "SELECT certificate_id,certificate FROM planar_certificates ORDER BY certificate_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            for row in rows {
                let (stated, certificate) = row?;
                if stated.as_slice() != unknotdb::util::sha256(&certificate) {
                    return Err("planar certificate hash mismatch".into());
                }
            }
        }
        Ok(())
    }

    /// Replay every production proof edge from the stored source checkpoint
    /// to the stored target checkpoint. This is intentionally a publication /
    /// regression gate rather than a hot-path startup check.
    pub fn verify_full_replay(&self) -> Result<usize> {
        let sql = if self.schema_version == 0 {
            "SELECT e.edge_id,e.cc_cost,e.first_action,e.program_version,e.program,\
                    sr.encoding,tr.encoding,0,0,NULL \
             FROM edges e \
             JOIN representations sr ON sr.node_id=e.source_node \
             JOIN representations tr ON tr.node_id=e.target_node \
             ORDER BY e.edge_id"
        } else if self.schema_version < 3 {
            "SELECT e.edge_id,e.cc_cost,e.first_action,p.program_version,p.program,\
                    sr.encoding,tr.encoding,0,0,NULL \
             FROM edges e JOIN programs p ON p.program_id=e.program_id \
             JOIN representations sr ON sr.node_id=e.source_node \
             JOIN representations tr ON tr.node_id=e.target_node \
             ORDER BY e.edge_id"
        } else {
            "SELECT e.edge_id,e.cc_cost,e.first_action,p.program_version,p.program,\
                    sr.encoding,tr.encoding,e.program_anchor_x,e.program_anchor_y, \
                    e.certificate_id \
             FROM edges e JOIN programs p ON p.program_id=e.program_id \
             JOIN representations sr ON sr.node_id=e.source_node \
             JOIN representations tr ON tr.node_id=e.target_node \
             ORDER BY e.edge_id"
        };
        let meta = read_meta(&self.conn)?;
        if meta.synthetic {
            return Err("synthetic snapshots have no replayable proof contract".into());
        }
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, Vec<u8>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<Vec<u8>>>(9)?,
            ))
        })?;
        let mut count = 0_usize;
        for row in rows {
            let (
                edge_id,
                cc,
                first,
                version,
                program,
                source,
                target,
                anchor_x,
                anchor_y,
                certificate_id,
            ) = row?;
            let source_representation =
                representation::BraidRepresentation::decode_storage(&source)?;
            let source = source_representation.encode()?;
            let target = representation::BraidRepresentation::decode_storage(&target)?.encode()?;
            let program_version = u32_from_i64(version, "program_version")?;
            let program = if self.schema_version >= 3
                && program_version == representation::CheckpointedProofProgram::VERSION
            {
                let template = representation::CheckpointedProofProgram::decode(&program)?;
                if template.contains_planar_certificate() {
                    if anchor_x != 0 || anchor_y != 0 {
                        return Err("planar certificate program has a non-zero anchor".into());
                    }
                    program
                } else {
                    representation::AnchoredProofProgram {
                        template,
                        anchor_x: u16::try_from(anchor_x)
                            .map_err(|_| "program anchor_x does not fit u16")?,
                        anchor_y: u32_from_i64(anchor_y, "program_anchor_y")?,
                    }
                    .materialize(&source_representation)?
                    .encode()?
                }
            } else {
                program
            };
            let certificate = certificate_id
                .as_deref()
                .map(|id| -> Result<Option<Vec<u8>>> {
                    if !table_exists(&self.conn, "planar_certificates")? {
                        return Ok(None);
                    }
                    Ok(self
                        .conn
                        .query_row(
                            "SELECT certificate FROM planar_certificates WHERE certificate_id=?1",
                            [id],
                            |row| row.get(0),
                        )
                        .optional()?)
                })
                .transpose()?
                .flatten();
            representation::verify_edge_program_with_certificate(
                &source,
                &target,
                &program,
                program_version,
                u64_from_i64(first, "first_action")?,
                u8_from_i64(cc, "cc_cost")?,
                certificate.as_deref(),
            )
            .map_err(|error| format!("edge {} full replay failed: {}", edge_id, error))?;
            count += 1;
        }
        Ok(count)
    }
}

/// Write an immutable snapshot through a temporary file and publish it with one
/// rename. Node and edge iterators are consumed streaming; building a 10 GB
/// snapshot does not require holding a second 10 GB object graph in RAM.
pub fn write_snapshot_atomic<N, E>(
    final_path: impl AsRef<Path>,
    meta: &SnapshotMeta,
    nodes: N,
    edges: E,
) -> Result<()>
where
    N: IntoIterator<Item = NodeRecord>,
    E: IntoIterator<Item = EdgeRecord>,
{
    let final_path = final_path.as_ref();
    meta.validate()?;
    let parent = final_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp_path = unique_temp_path(final_path);

    let result = (|| -> Result<()> {
        let conn = Connection::open_with_flags(
            &temp_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // Enabling deferred FKs while bulk-loading edges makes SQLite revisit
        // every forward node->edge reference as its edge appears. The snapshot
        // is immutable, so the indexed checks below validate all relationships
        // once after both tables exist.
        conn.execute_batch(
            "PRAGMA page_size=4096;
             PRAGMA journal_mode=OFF;
             PRAGMA synchronous=OFF;
             PRAGMA temp_store=MEMORY;
             PRAGMA foreign_keys=OFF;",
        )?;
        conn.execute_batch(SCHEMA_SQL)?;
        conn.execute_batch("BEGIN IMMEDIATE")?;

        insert_meta(&conn, meta)?;
        {
            let mut node_stmt = conn.prepare_cached(
                "INSERT INTO nodes( \
                    node_id, role_mask, u_upper_bound, unknot_route_rank, \
                    acs10, acs_route_rank, \
                    next_unknot_edge, next_unknot_target, next_unknot_cc_cost, \
                    first_unknot_action, next_acs10_edge, next_acs10_target, \
                    next_acs10_cc_cost, first_acs10_action \
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            )?;
            let mut key_stmt = conn.prepare_cached(
                "INSERT INTO node_keys(rep_key, node_id, key_kind) VALUES (?1, ?2, ?3)",
            )?;
            let mut rep_stmt = conn
                .prepare_cached("INSERT INTO representations(node_id, encoding) VALUES (?1, ?2)")?;
            let mut stop_stmt = conn.prepare_cached(
                "INSERT INTO policy_stops( \
                    node_id, stop_kind, preferred_cc_action, audit_sha256 \
                 ) VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut expected_node_id = 0_u32;
            for node in nodes {
                validate_node_record(&node)?;
                validate_node_for_meta(&node, meta)?;
                if node.node_id != expected_node_id {
                    return Err(format!(
                        "writer node IDs must be dense and ordered: expected {}, found {}",
                        expected_node_id, node.node_id
                    )
                    .into());
                }
                expected_node_id = expected_node_id.checked_add(1).ok_or("too many nodes")?;
                let u = node.next_unknot;
                let a = node.next_acs10;
                node_stmt.execute(params![
                    i64::from(node.node_id),
                    i64::from(node.role_mask),
                    node.u_upper_bound.map(i64::from),
                    node.unknot_route_rank.map(i64_from_u64).transpose()?,
                    node.acs10.map(i64::from),
                    node.acs_route_rank.map(i64_from_u64).transpose()?,
                    u.map(|s| i64::from(s.edge_id)),
                    u.map(|s| i64::from(s.target_node)),
                    u.map(|s| i64::from(s.cc_cost)),
                    u.map(|s| i64_from_u64(s.first_action)).transpose()?,
                    a.map(|s| i64::from(s.edge_id)),
                    a.map(|s| i64::from(s.target_node)),
                    a.map(|s| i64::from(s.cc_cost)),
                    a.map(|s| i64_from_u64(s.first_action)).transpose()?,
                ])?;
                let key_kind = if meta.synthetic {
                    0_i64
                } else {
                    let decoded = representation::BraidRepresentation::decode(&node.encoding)?;
                    if decoded.is_normalized()? {
                        0_i64
                    } else {
                        1_i64
                    }
                };
                key_stmt.execute(params![
                    node.rep_key.as_slice(),
                    i64::from(node.node_id),
                    key_kind
                ])?;
                let stored_encoding = if meta.synthetic {
                    node.encoding
                } else {
                    representation::BraidRepresentation::decode(&node.encoding)?.encode_storage()?
                };
                rep_stmt.execute(params![i64::from(node.node_id), stored_encoding])?;
                if let Some(stop) = node.policy_stop {
                    let (kind, action, audit) = match stop {
                        PolicyStopAttestation::PreferredCrossingChange {
                            action,
                            audit_sha256,
                        } => (0_i64, Some(i64_from_u64(action)?), audit_sha256),
                        PolicyStopAttestation::Terminal { audit_sha256 } => {
                            (1_i64, None, audit_sha256)
                        }
                        PolicyStopAttestation::CapacityFallback { audit_sha256 } => {
                            (2_i64, None, audit_sha256)
                        }
                    };
                    stop_stmt.execute(params![
                        i64::from(node.node_id),
                        kind,
                        action,
                        audit.as_slice(),
                    ])?;
                }
            }
        }
        {
            let mut program_stmt = conn.prepare_cached(
                "INSERT INTO programs( \
                    program_id, program_version, program, program_sha256 \
                 ) VALUES (?1,?2,?3,?4)",
            )?;
            let mut edge_stmt = conn.prepare_cached(
                "INSERT INTO edges( \
                    edge_id, source_node, target_node, cc_cost, first_action, \
                    program_id, program_anchor_x, program_anchor_y, certificate_id \
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            )?;
            let mut certificate_stmt = conn.prepare_cached(
                "INSERT OR IGNORE INTO planar_certificates( \
                    certificate_id, certificate_version, certificate \
                 ) VALUES (?1,0,?2)",
            )?;
            let mut source_rep_stmt =
                conn.prepare_cached("SELECT encoding FROM representations WHERE node_id=?1")?;
            let mut program_ids: HashMap<u32, HashMap<Vec<u8>, u32>> = HashMap::new();
            let mut next_program_id = 0_u32;
            for edge in edges {
                validate_edge_record(&edge)?;
                if let Some(certificate) = edge.certificate.as_deref() {
                    let certificate_id = unknotdb::util::sha256(certificate);
                    if edge.certificate_id != Some(certificate_id) {
                        return Err(format!(
                            "edge {} certificate_id does not hash its sidecar",
                            edge.edge_id
                        )
                        .into());
                    }
                    certificate_stmt.execute(params![certificate_id.as_slice(), certificate])?;
                    let stored: Vec<u8> = conn.query_row(
                        "SELECT certificate FROM planar_certificates WHERE certificate_id=?1",
                        [certificate_id.as_slice()],
                        |row| row.get(0),
                    )?;
                    if stored != certificate {
                        return Err("planar certificate hash collision".into());
                    }
                }
                validate_edge_for_meta(&conn, &edge, meta)?;
                if edge.validator_version != meta.validator_version {
                    return Err(format!(
                        "edge {} validator `{}` differs from snapshot validator `{}`",
                        edge.edge_id, edge.validator_version, meta.validator_version
                    )
                    .into());
                }
                let (stored_program, anchor_x, anchor_y) = if meta.synthetic
                    || edge.program_version != representation::CheckpointedProofProgram::VERSION
                {
                    (edge.program.clone(), 0_u16, 0_u32)
                } else {
                    let stored_source: Vec<u8> = source_rep_stmt
                        .query_row([i64::from(edge.source_node)], |row| row.get(0))?;
                    let source =
                        representation::BraidRepresentation::decode_storage(&stored_source)?;
                    let exact = representation::CheckpointedProofProgram::decode(&edge.program)?;
                    if exact.contains_planar_certificate() {
                        (edge.program.clone(), 0, 0)
                    } else {
                        let anchored = exact.anchor_template(&source)?;
                        let materialized = anchored.materialize(&source)?.encode()?;
                        if materialized != edge.program {
                            return Err(format!(
                                "edge {} anchored template changed its exact program",
                                edge.edge_id
                            )
                            .into());
                        }
                        (
                            anchored.template.encode()?,
                            anchored.anchor_x,
                            anchored.anchor_y,
                        )
                    }
                };
                let version_programs = program_ids.entry(edge.program_version).or_default();
                let program_id = if let Some(id) = version_programs.get(&stored_program) {
                    *id
                } else {
                    let id = next_program_id;
                    next_program_id = next_program_id
                        .checked_add(1)
                        .ok_or("too many distinct programs")?;
                    let program_hash = unknotdb::util::sha256(&stored_program);
                    program_stmt.execute(params![
                        i64::from(id),
                        i64::from(edge.program_version),
                        &stored_program,
                        program_hash.as_slice(),
                    ])?;
                    version_programs.insert(stored_program, id);
                    id
                };
                edge_stmt.execute(params![
                    i64::from(edge.edge_id),
                    i64::from(edge.source_node),
                    i64::from(edge.target_node),
                    i64::from(edge.cc_cost),
                    i64_from_u64(edge.first_action)?,
                    i64::from(program_id),
                    i64::from(anchor_x),
                    i64::from(anchor_y),
                    edge.certificate_id.map(|v| v.to_vec()),
                ])?;
            }
        }

        validate_connection(&conn)?;
        conn.execute_batch("COMMIT; PRAGMA optimize; VACUUM; PRAGMA query_only=ON;")?;
        let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if check != "ok" {
            return Err(format!("SQLite quick_check failed: {}", check).into());
        }
        conn.close().map_err(|(_, e)| e)?;
        // The build deliberately uses synchronous=OFF because the temporary
        // file is disposable. Flush the completed image once before the
        // visibility-changing rename.
        File::open(&temp_path)?.sync_all()?;
        fs::rename(&temp_path, final_path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

pub fn synthetic_node_count(conn: &Connection) -> Result<usize> {
    scalar_usize(conn, "SELECT count(*) FROM nodes")
}

pub fn synthetic_key(id: u64) -> RepKey {
    let mut key = [0u8; 32];
    key[..8].copy_from_slice(&id.to_le_bytes());
    let mut state = id ^ 0x9e37_79b9_7f4a_7c15;
    for chunk in key[8..].chunks_exact_mut(8) {
        state = splitmix64(state);
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    key
}

pub fn synthetic_nodes(count: u32) -> impl Iterator<Item = NodeRecord> {
    (0..count).map(|id| {
        let route = (id > 0).then(|| RouteStep {
            edge_id: id - 1,
            target_node: 0,
            cc_cost: 1,
            first_action: u64::from(id),
        });
        NodeRecord {
            rep_key: synthetic_key(u64::from(id)),
            node_id: id,
            role_mask: ROLE_CORE,
            encoding: u64::from(id).to_le_bytes().to_vec(),
            u_upper_bound: Some(if id == 0 { 0 } else { 1 }),
            unknot_route_rank: Some(u64::from(id)),
            acs10: Some(if id == 0 { 0 } else { 20 + id % 97 }),
            acs_route_rank: Some(u64::from(id)),
            next_unknot: route,
            next_acs10: route,
            policy_stop: None,
        }
    })
}

pub fn synthetic_edges(count: u32) -> impl Iterator<Item = EdgeRecord> {
    (1..count).map(|source| EdgeRecord {
        edge_id: source - 1,
        source_node: source,
        target_node: 0,
        cc_cost: 1,
        first_action: u64::from(source),
        program_version: 0,
        program: format!("SYNTHETIC-XC node={source}").into_bytes(),
        certificate_id: None,
        certificate: None,
        validator_version: "synthetic-structural-only-v0".into(),
    })
}

fn insert_meta(conn: &Connection, meta: &SnapshotMeta) -> Result<()> {
    let created_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let values = [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("snapshot_version", meta.snapshot_version.clone()),
        ("key_spec", meta.key_spec.clone()),
        ("normalizer_version", meta.normalizer_version.clone()),
        ("representation_codec", meta.representation_codec.clone()),
        ("action_codec", meta.action_codec.clone()),
        (
            "program_binding_codec",
            representation::ANCHORED_PROGRAM_CODEC.into(),
        ),
        ("validator_version", meta.validator_version.clone()),
        ("source_generation", meta.source_generation.clone()),
        (
            "policy_model_id",
            meta.policy_model_id
                .clone()
                .unwrap_or_else(|| "none".into()),
        ),
        (
            "policy_objective_ratio",
            meta.policy_objective_ratio
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".into()),
        ),
        (
            "policy_adapter_version",
            meta.policy_adapter_version
                .clone()
                .unwrap_or_else(|| "none".into()),
        ),
        (
            "controller_initial_state",
            meta.controller_initial_state
                .clone()
                .unwrap_or_else(|| "none".into()),
        ),
        ("synthetic", if meta.synthetic { "1" } else { "0" }.into()),
        ("created_unix_ms", created_ms.to_string()),
    ];
    let mut stmt = conn.prepare_cached("INSERT INTO meta(key, value) VALUES (?1, ?2)")?;
    for (key, value) in values {
        if value.is_empty() {
            return Err(format!("metadata `{}` is empty", key).into());
        }
        stmt.execute(params![key, value])?;
    }
    Ok(())
}

fn read_meta(conn: &Connection) -> Result<SnapshotMeta> {
    let get = |key: &str| -> Result<String> {
        conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .map_err(Into::into)
    };
    let schema: u32 = get("schema_version")?.parse()?;
    if schema > SCHEMA_VERSION {
        return Err(format!(
            "unsupported snapshot schema {}; runtime supports through {}",
            schema, SCHEMA_VERSION
        )
        .into());
    }
    let meta = SnapshotMeta {
        snapshot_version: get("snapshot_version")?,
        key_spec: get("key_spec")?,
        normalizer_version: get("normalizer_version")?,
        representation_codec: get("representation_codec")?,
        action_codec: get("action_codec")?,
        validator_version: conn
            .query_row(
                "SELECT value FROM meta WHERE key='validator_version'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| representation::VALIDATOR_VERSION.into()),
        source_generation: get("source_generation")?,
        policy_model_id: optional_meta(&get("policy_model_id")?),
        policy_objective_ratio: match get("policy_objective_ratio")?.as_str() {
            "none" => None,
            value => Some(value.parse()?),
        },
        policy_adapter_version: optional_meta(&get("policy_adapter_version")?),
        controller_initial_state: optional_meta(&get("controller_initial_state")?),
        synthetic: match get("synthetic")?.as_str() {
            "0" => false,
            "1" => true,
            other => return Err(format!("invalid synthetic metadata `{}`", other).into()),
        },
    };
    meta.validate()?;
    Ok(meta)
}

fn snapshot_schema_version(conn: &Connection) -> Result<u32> {
    let value: String = conn.query_row(
        "SELECT value FROM meta WHERE key='schema_version'",
        [],
        |row| row.get(0),
    )?;
    Ok(value.parse()?)
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn optional_meta(value: &str) -> Option<String> {
    (value != "none").then(|| value.to_owned())
}

fn validate_header(conn: &Connection) -> Result<()> {
    let application_id: i64 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if application_id != 1_430_996_034 {
        return Err(format!("not an unknotdb snapshot (application_id={application_id})").into());
    }
    let _ = read_meta(conn)?;
    Ok(())
}

fn validate_connection(conn: &Connection) -> Result<()> {
    validate_header(conn)?;
    let meta = read_meta(conn)?;
    validate_dense_ids(conn, "nodes", "node_id")?;
    validate_dense_ids(conn, "edges", "edge_id")?;
    let schema = snapshot_schema_version(conn)?;
    if schema >= 3 {
        let binding: String = conn.query_row(
            "SELECT value FROM meta WHERE key='program_binding_codec'",
            [],
            |row| row.get(0),
        )?;
        if binding != representation::ANCHORED_PROGRAM_CODEC {
            return Err(format!("unsupported program binding codec `{binding}`").into());
        }
    }
    if table_exists(conn, "planar_certificates")? {
        let orphan: Option<Vec<u8>> = conn
            .query_row(
                "SELECT c.certificate_id FROM planar_certificates c LEFT JOIN edges e \
                 ON e.certificate_id=c.certificate_id WHERE e.edge_id IS NULL LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if orphan.is_some() {
            return Err("planar certificate is not used by an edge".into());
        }
    }
    if schema >= 2 {
        let node_count = scalar_usize(conn, "SELECT count(*) FROM nodes")?;
        let key_count = scalar_usize(conn, "SELECT count(*) FROM node_keys")?;
        if key_count != node_count {
            return Err(format!(
                "snapshot has {} node keys for {} nodes",
                key_count, node_count
            )
            .into());
        }
        let bad_key_id: Option<i64> = conn
            .query_row(
                "SELECT node_id FROM node_keys WHERE node_id<0 OR node_id>=?1 LIMIT 1",
                [i64::try_from(node_count)?],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = bad_key_id {
            return Err(format!("node key points outside the dense arena: {}", id).into());
        }
        let duplicate_key_id: Option<i64> = conn
            .query_row(
                "SELECT node_id FROM node_keys GROUP BY node_id HAVING count(*)>1 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = duplicate_key_id {
            return Err(format!("node {} has multiple rep_keys", id).into());
        }
    }

    let bad_range: Option<i64> = conn
        .query_row(
            "SELECT node_id FROM nodes WHERE \
                 node_id>4294967295 OR \
                 coalesce(u_upper_bound,0)>4294967295 OR \
                 coalesce(acs10,0)>4294967295 OR \
                 coalesce(next_unknot_edge,0)>4294967295 OR \
                 coalesce(next_unknot_target,0)>4294967295 OR \
                 coalesce(next_acs10_edge,0)>4294967295 OR \
                 coalesce(next_acs10_target,0)>4294967295 \
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = bad_range {
        return Err(format!("node {} has a field outside the v0 integer range", id).into());
    }
    let edge_range_sql = if schema == 0 {
        "SELECT edge_id FROM edges WHERE \
             edge_id>4294967295 OR source_node>4294967295 OR \
             target_node>4294967295 OR program_version>4294967295 OR \
             length(program)>4294967295 LIMIT 1"
    } else {
        "SELECT e.edge_id FROM edges e JOIN programs p ON p.program_id=e.program_id WHERE \
             e.edge_id>4294967295 OR e.source_node>4294967295 OR \
             e.target_node>4294967295 OR e.program_id>4294967295 OR \
             p.program_version>4294967295 OR length(p.program)>4294967295 LIMIT 1"
    };
    let bad_edge_range: Option<i64> = conn
        .query_row(edge_range_sql, [], |row| row.get(0))
        .optional()?;
    if let Some(id) = bad_edge_range {
        return Err(format!("edge {} has a field outside the v0 integer range", id).into());
    }
    if schema >= 1 {
        validate_dense_ids(conn, "programs", "program_id")?;
        let missing_program: Option<i64> = conn
            .query_row(
                "SELECT e.edge_id FROM edges e LEFT JOIN programs p \
                 ON p.program_id=e.program_id WHERE p.program_id IS NULL LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = missing_program {
            return Err(format!("edge {} has no proof program", id).into());
        }
        let orphan_program: Option<i64> = conn
            .query_row(
                "SELECT p.program_id FROM programs p LEFT JOIN edges e \
                 ON e.program_id=p.program_id WHERE e.edge_id IS NULL LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = orphan_program {
            return Err(format!("program {} is not used by an edge", id).into());
        }
        let duplicate_program: Option<i64> = conn
            .query_row(
                "SELECT min(program_id) FROM programs \
                 GROUP BY program_version, program HAVING count(*)>1 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = duplicate_program {
            return Err(format!("program {} duplicates another dictionary entry", id).into());
        }
    }
    if schema >= 3 {
        let bad_anchor: Option<i64> = conn
            .query_row(
                "SELECT edge_id FROM edges WHERE program_anchor_x<0 OR \
                 program_anchor_x>65535 OR program_anchor_y<0 OR \
                 program_anchor_y>4294967295 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = bad_anchor {
            return Err(format!("edge {} has an invalid program anchor", id).into());
        }
    }

    // `PRAGMA foreign_key_check` repeatedly scans the two forward
    // node->edge references on large snapshots. These indexed joins express
    // the same three relationships in linear/indexed passes.
    let bad_rep: Option<i64> = conn
        .query_row(
            "SELECT r.node_id FROM representations r \
             LEFT JOIN nodes n ON n.node_id=r.node_id \
             WHERE n.node_id IS NULL LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = bad_rep {
        return Err(format!("representation {} has no node", id).into());
    }
    let missing_rep: Option<i64> = conn
        .query_row(
            "SELECT n.node_id FROM nodes n \
             LEFT JOIN representations r ON r.node_id=n.node_id \
             WHERE r.node_id IS NULL LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = missing_rep {
        return Err(format!("node {} has no representation", id).into());
    }
    let orphan_stop: Option<i64> = conn
        .query_row(
            "SELECT s.node_id FROM policy_stops s \
             LEFT JOIN nodes n ON n.node_id=s.node_id \
             WHERE n.node_id IS NULL LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = orphan_stop {
        return Err(format!("policy stop {} has no node", id).into());
    }
    let stop_count = scalar_usize(conn, "SELECT count(*) FROM policy_stops")?;
    let node_count = scalar_usize(conn, "SELECT count(*) FROM nodes")?;
    if meta.synthetic && stop_count != 0 {
        return Err("synthetic snapshot contains policy stopping attestations".into());
    }
    let schema = snapshot_schema_version(conn)?;
    if !meta.synthetic && schema < 4 && stop_count != node_count {
        return Err(format!(
            "production snapshot has {} policy stops for {} vertices",
            stop_count, node_count
        )
        .into());
    }
    if !meta.synthetic && schema >= 4 && stop_count > node_count {
        return Err("production snapshot has more policy stops than vertices".into());
    }
    if schema >= 4 && !meta.synthetic {
        let bad_key_kind: Option<i64> = conn
            .query_row(
                "SELECT node_id FROM node_keys WHERE key_kind NOT IN (0,1) LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = bad_key_kind {
            return Err(format!("node {} has an invalid key kind", id).into());
        }
        let mut stmt = conn.prepare(
            "SELECT k.node_id,k.rep_key,k.key_kind,r.encoding, \
                    EXISTS(SELECT 1 FROM policy_stops s WHERE s.node_id=k.node_id) \
             FROM node_keys k JOIN representations r ON r.node_id=k.node_id \
             ORDER BY k.node_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?;
        for row in rows {
            let (node_id, raw_key, key_kind, encoding, has_stop) = row?;
            let key = rep_key_from_slice(&raw_key)?;
            let representation = representation::BraidRepresentation::decode_storage(&encoding)?;
            let normalized = representation.normalize()?;
            let is_normalized = representation.is_normalized()?;
            let expected_kind = if is_normalized { 0 } else { 1 };
            if key_kind != expected_kind {
                return Err(format!(
                    "node {} key kind disagrees with its exact representation",
                    node_id
                )
                .into());
            }
            let expected_key = if is_normalized {
                normalized.key
            } else {
                representation.exact_checkpoint_key()?
            };
            if key != expected_key {
                return Err(format!("node {} checkpoint key mismatch", node_id).into());
            }
            if has_stop && !is_normalized {
                return Err(
                    format!("node {} has a policy stop but is not normalized", node_id).into(),
                );
            }
        }
    }
    let bad_edge: Option<i64> = conn
        .query_row(
            "SELECT e.edge_id FROM edges e \
             LEFT JOIN nodes s ON s.node_id=e.source_node \
             LEFT JOIN nodes t ON t.node_id=e.target_node \
             WHERE s.node_id IS NULL OR t.node_id IS NULL LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = bad_edge {
        return Err(format!("edge {} has a missing endpoint", id).into());
    }

    let bad_partial: Option<i64> = conn
        .query_row(
            "SELECT node_id FROM nodes WHERE \
             ((next_unknot_edge IS NULL) + (next_unknot_target IS NULL) + \
              (next_unknot_cc_cost IS NULL) + (first_unknot_action IS NULL)) NOT IN (0,4) \
             OR ((next_acs10_edge IS NULL) + (next_acs10_target IS NULL) + \
                 (next_acs10_cc_cost IS NULL) + (first_acs10_action IS NULL)) NOT IN (0,4) \
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = bad_partial {
        return Err(format!("node {} has a partial materialized route", id).into());
    }

    let bad_u: Option<i64> = conn
        .query_row(
            "SELECT n.node_id \
             FROM nodes n \
             LEFT JOIN edges e ON e.edge_id=n.next_unknot_edge \
             LEFT JOIN nodes t ON t.node_id=e.target_node \
             WHERE n.next_unknot_edge IS NOT NULL AND ( \
                 e.edge_id IS NULL OR e.source_node<>n.node_id OR \
                 n.next_unknot_target<>e.target_node OR \
                 n.next_unknot_cc_cost<>e.cc_cost OR \
                 n.first_unknot_action<>e.first_action OR \
                 n.u_upper_bound IS NULL OR t.u_upper_bound IS NULL OR \
                 n.u_upper_bound<>t.u_upper_bound+e.cc_cost OR \
                 n.unknot_route_rank IS NULL OR t.unknot_route_rank IS NULL OR \
                 t.unknot_route_rank>=n.unknot_route_rank \
             ) LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = bad_u {
        return Err(format!("node {} violates the unknotting-route invariant", id).into());
    }

    let bad_acs: Option<i64> = conn
        .query_row(
            "SELECT n.node_id \
             FROM nodes n \
             LEFT JOIN edges e ON e.edge_id=n.next_acs10_edge \
             LEFT JOIN nodes t ON t.node_id=e.target_node \
             WHERE n.next_acs10_edge IS NOT NULL AND ( \
                 e.edge_id IS NULL OR e.source_node<>n.node_id OR \
                 n.next_acs10_target<>e.target_node OR \
                 n.next_acs10_cc_cost<>e.cc_cost OR \
                 n.first_acs10_action<>e.first_action OR \
                 n.acs10 IS NULL OR t.acs10 IS NULL OR \
                 t.acs10>=n.acs10+e.cc_cost OR \
                 n.acs_route_rank IS NULL OR t.acs_route_rank IS NULL OR \
                 t.acs_route_rank>=n.acs_route_rank \
             ) LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = bad_acs {
        return Err(format!("node {} violates the ACS10-route invariant", id).into());
    }
    Ok(())
}

fn validate_dense_ids(conn: &Connection, table: &str, column: &str) -> Result<()> {
    let sql = format!(
        "SELECT count(*), coalesce(min({column}),0), coalesce(max({column}),-1) FROM {table}"
    );
    let (count, min, max): (i64, i64, i64) =
        conn.query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    if count == 0 {
        if table == "nodes" {
            return Err("snapshot contains no nodes".into());
        }
        return Ok(());
    }
    if min != 0 || max != count - 1 {
        return Err(format!(
            "{}.{} must be dense 0..{}, got min={} max={} count={}",
            table,
            column,
            count - 1,
            min,
            max,
            count
        )
        .into());
    }
    Ok(())
}

fn validate_node_record(node: &NodeRecord) -> Result<()> {
    if node.role_mask == 0 || node.role_mask > 7 {
        return Err(format!("node {} has invalid role_mask", node.node_id).into());
    }
    for route in [node.next_unknot, node.next_acs10].into_iter().flatten() {
        let _ = i64_from_u64(route.first_action)?;
        if route.cc_cost > 1 {
            return Err(format!("node {} route has cc_cost > 1", node.node_id).into());
        }
    }
    if let Some(rank) = node.acs_route_rank {
        let _ = i64_from_u64(rank)?;
    }
    if let Some(rank) = node.unknot_route_rank {
        let _ = i64_from_u64(rank)?;
    }
    if let Some(PolicyStopAttestation::PreferredCrossingChange { action, .. }) = node.policy_stop {
        let _ = i64_from_u64(action)?;
    }
    Ok(())
}

fn validate_edge_record(edge: &EdgeRecord) -> Result<()> {
    if edge.cc_cost > 1 {
        return Err(format!("edge {} has cc_cost > 1", edge.edge_id).into());
    }
    let _ = i64_from_u64(edge.first_action)?;
    if edge.validator_version.is_empty() {
        return Err(format!("edge {} has no validator version", edge.edge_id).into());
    }
    if edge.certificate.is_some() && edge.certificate_id.is_none() {
        return Err(format!("edge {} has a certificate without an id", edge.edge_id).into());
    }
    Ok(())
}

fn validate_node_for_meta(node: &NodeRecord, meta: &SnapshotMeta) -> Result<()> {
    if meta.synthetic {
        if node.policy_stop.is_some() {
            return Err(format!("synthetic node {} has a policy stop", node.node_id).into());
        }
        return Ok(());
    }
    let decoded = representation::BraidRepresentation::decode(&node.encoding)
        .map_err(|error| format!("node {} representation: {}", node.node_id, error))?;
    let normalized = decoded.normalize()?;
    let is_normalized = !normalized.witness.mirrored
        && normalized.witness.rotate_word_left == 0
        && normalized.encoded == node.encoding;
    if node.policy_stop.is_some() && !is_normalized {
        return Err(format!(
            "node {} has a policy/reducer attestation but is not normalized",
            node.node_id
        )
        .into());
    }
    let expected_key = if is_normalized {
        normalized.key
    } else {
        decoded.exact_checkpoint_key()?
    };
    if expected_key != node.rep_key {
        return Err(format!(
            "node {} key does not match its checkpoint kind",
            node.node_id
        )
        .into());
    }
    if !decoded.is_knot_closure()? {
        return Err(format!(
            "node {} representation does not close to one knot",
            node.node_id
        )
        .into());
    }
    for route in [node.next_unknot, node.next_acs10].into_iter().flatten() {
        representation::SemanticAction::decode_u63(route.first_action).map_err(|error| {
            format!(
                "node {} has an invalid first action: {}",
                node.node_id, error
            )
        })?;
    }
    let Some(policy_stop) = node.policy_stop else {
        // Exact proof checkpoints need no policy assertion. If unnormalized,
        // their domain-separated key prevents accidental canonical lookup or
        // mirror-orbit merging. Their graph edges remain fully replay-checked.
        return Ok(());
    };
    match policy_stop {
        PolicyStopAttestation::PreferredCrossingChange { action, .. } => {
            let action = representation::SemanticAction::decode_u63(action).map_err(|error| {
                format!(
                    "node {} has an invalid preferred CC: {}",
                    node.node_id, error
                )
            })?;
            if !action.is_crossing_change() {
                return Err(
                    format!("node {} preferred policy action is not CC", node.node_id).into(),
                );
            }
            action.apply(&decoded).map_err(|error| {
                format!("node {} preferred CC is illegal: {}", node.node_id, error)
            })?;
        }
        PolicyStopAttestation::Terminal { .. } => {
            if decoded.strands != 1 || !decoded.word.is_empty() {
                return Err(format!("node {} falsely claims to be terminal", node.node_id).into());
            }
        }
        PolicyStopAttestation::CapacityFallback { .. } => {}
    }
    Ok(())
}

fn validate_edge_for_meta(conn: &Connection, edge: &EdgeRecord, meta: &SnapshotMeta) -> Result<()> {
    if meta.synthetic {
        return Ok(());
    }
    if edge.program_version > representation::CheckpointedProofProgram::VERSION {
        return Err(format!(
            "edge {} uses unsupported program version {}",
            edge.edge_id, edge.program_version
        )
        .into());
    }
    if edge.validator_version != representation::VALIDATOR_VERSION {
        return Err(format!(
            "edge {} validator version is `{}`, expected `{}`",
            edge.edge_id,
            edge.validator_version,
            representation::VALIDATOR_VERSION
        )
        .into());
    }
    representation::SemanticAction::decode_u63(edge.first_action).map_err(|error| {
        format!(
            "edge {} has an invalid first action: {}",
            edge.edge_id, error
        )
    })?;
    let load_encoding = |node_id: NodeId| -> Result<Vec<u8>> {
        conn.query_row(
            "SELECT encoding FROM representations WHERE node_id=?1",
            [i64::from(node_id)],
            |row| row.get(0),
        )
        .map_err(|error| {
            format!(
                "edge {} cannot load checkpoint node {}: {}",
                edge.edge_id, node_id, error
            )
            .into()
        })
    };
    let source =
        representation::BraidRepresentation::decode_storage(&load_encoding(edge.source_node)?)?
            .encode()?;
    let target =
        representation::BraidRepresentation::decode_storage(&load_encoding(edge.target_node)?)?
            .encode()?;
    representation::verify_edge_program_with_certificate(
        &source,
        &target,
        &edge.program,
        edge.program_version,
        edge.first_action,
        edge.cc_cost,
        edge.certificate.as_deref(),
    )
    .map_err(|error| -> Box<dyn Error + Send + Sync> {
        format!("edge {} replay failed: {}", edge.edge_id, error).into()
    })?;
    Ok(())
}

fn hot_route_from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<HotRoute> {
    let node_id = row.get::<_, i64>(offset)? as u32;
    let u_upper_bound = row.get::<_, Option<i64>>(offset + 1)?.map(|v| v as u32);
    let acs10 = row.get::<_, Option<i64>>(offset + 2)?.map(|v| v as u32);
    let next_unknot = route_step_from_row(row, offset + 3)?;
    let next_acs10 = route_step_from_row(row, offset + 7)?;
    Ok(HotRoute {
        node_id,
        u_upper_bound,
        acs10,
        next_unknot,
        next_acs10,
    })
}

fn route_step_from_row(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<Option<RouteStep>> {
    let edge: Option<i64> = row.get(offset)?;
    let target: Option<i64> = row.get(offset + 1)?;
    let cost: Option<i64> = row.get(offset + 2)?;
    let action: Option<i64> = row.get(offset + 3)?;
    Ok(match (edge, target, cost, action) {
        (Some(edge_id), Some(target_node), Some(cc_cost), Some(first_action)) => Some(RouteStep {
            edge_id: edge_id as u32,
            target_node: target_node as u32,
            cc_cost: cc_cost as u8,
            first_action: first_action as u64,
        }),
        (None, None, None, None) => None,
        _ => return Err(rusqlite::Error::InvalidQuery),
    })
}

fn scalar_usize(conn: &Connection, sql: &str) -> Result<usize> {
    let value: i64 = conn.query_row(sql, [], |row| row.get(0))?;
    Ok(value.try_into()?)
}

fn rep_key_from_slice(bytes: &[u8]) -> Result<RepKey> {
    bytes
        .try_into()
        .map_err(|_| format!("rep_key has {} bytes, expected 32", bytes.len()).into())
}

fn i64_from_u64(value: u64) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| format!("{} does not fit in SQLite INTEGER", value).into())
}

fn u64_from_i64(value: i64, field: &str) -> Result<u64> {
    value
        .try_into()
        .map_err(|_| format!("{} is negative in {}", value, field).into())
}

fn u32_from_i64(value: i64, field: &str) -> Result<u32> {
    value
        .try_into()
        .map_err(|_| format!("{} does not fit {}", value, field).into())
}

fn u8_from_i64(value: i64, field: &str) -> Result<u8> {
    value
        .try_into()
        .map_err(|_| format!("{} does not fit {}", value, field).into())
}

fn unique_temp_path(final_path: &Path) -> PathBuf {
    let pid = std::process::id();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = final_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("snapshot.db");
    final_path.with_file_name(format!(".{name}.tmp-{pid}-{stamp}"))
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn production_meta() -> SnapshotMeta {
        SnapshotMeta::braid_mirror_orbit_v1(
            "test",
            "test-policy:Q254:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
    }

    fn cc_stop(position: u32) -> PolicyStopAttestation {
        PolicyStopAttestation::PreferredCrossingChange {
            action: representation::SemanticAction::CrossingChange { position }
                .encode_u63()
                .unwrap(),
            audit_sha256: [7; 32],
        }
    }

    fn test_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unknotdb-runtime-{label}-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn all_three_lookup_modes_agree() {
        let path = test_path("lookup");
        write_snapshot_atomic(
            &path,
            &SnapshotMeta::synthetic(),
            synthetic_nodes(64),
            synthetic_edges(64),
        )
        .unwrap();

        let file = SqliteSnapshot::open_file(&path).unwrap();
        let memory = SqliteSnapshot::open_deserialized(&path).unwrap();
        let hash = GraphSnapshot::load_file(&path).unwrap();
        for id in 0..64 {
            let key = synthetic_key(id);
            let expected = hash.lookup(&key);
            assert_eq!(file.lookup(&key).unwrap(), expected);
            assert_eq!(memory.lookup(&key).unwrap(), expected);
        }
        let missing = synthetic_key(1_000_000);
        assert_eq!(file.lookup(&missing).unwrap(), None);
        assert_eq!(memory.lookup(&missing).unwrap(), None);
        assert_eq!(hash.lookup(&missing), None);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn live_graph_swap_keeps_old_readers_alive() {
        let first_path = test_path("swap-a");
        let second_path = test_path("swap-b");
        write_snapshot_atomic(
            &first_path,
            &SnapshotMeta::synthetic(),
            synthetic_nodes(2),
            synthetic_edges(2),
        )
        .unwrap();
        write_snapshot_atomic(
            &second_path,
            &SnapshotMeta::synthetic(),
            synthetic_nodes(3),
            synthetic_edges(3),
        )
        .unwrap();

        let live = LiveGraph::new(GraphSnapshot::load_file(&first_path).unwrap());
        let old_reader = live.load();
        live.publish(GraphSnapshot::load_file(&second_path).unwrap());
        assert_eq!(old_reader.node_count(), 2);
        assert_eq!(live.load().node_count(), 3);
        fs::remove_file(first_path).unwrap();
        fs::remove_file(second_path).unwrap();
    }

    #[test]
    fn schema_four_admits_exact_unnormalized_proof_checkpoint() {
        let path = test_path("exact-checkpoint");
        let representation = representation::BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![-1],
        };
        assert!(!representation.is_normalized().unwrap());
        let key = representation.exact_checkpoint_key().unwrap();
        let node = NodeRecord {
            rep_key: key,
            node_id: 0,
            role_mask: ROLE_CORE,
            encoding: representation.encode().unwrap(),
            u_upper_bound: None,
            unknot_route_rank: None,
            acs10: None,
            acs_route_rank: None,
            next_unknot: None,
            next_acs10: None,
            policy_stop: None,
        };
        write_snapshot_atomic(&path, &production_meta(), [node], []).unwrap();
        let snapshot = SqliteSnapshot::open_file(&path).unwrap();
        assert_eq!(snapshot.schema_version().unwrap(), 4);
        assert!(snapshot.lookup(&key).unwrap().is_some());
        let key_kind: i64 = snapshot
            .connection()
            .query_row(
                "SELECT key_kind FROM node_keys WHERE node_id=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(key_kind, 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn zero_cc_unknot_route_requires_equal_u_and_decreasing_rank() {
        let path = test_path("zero-cc-u-route");
        let mut nodes: Vec<_> = synthetic_nodes(2).collect();
        let mut edges: Vec<_> = synthetic_edges(2).collect();
        edges[0].cc_cost = 0;
        nodes[1].u_upper_bound = Some(0);
        nodes[1].next_unknot.as_mut().unwrap().cc_cost = 0;
        nodes[1].next_acs10.as_mut().unwrap().cc_cost = 0;
        write_snapshot_atomic(
            &path,
            &SnapshotMeta::synthetic(),
            nodes.clone(),
            edges.clone(),
        )
        .unwrap();
        fs::remove_file(&path).unwrap();

        nodes[1].unknot_route_rank = Some(0);
        let error =
            write_snapshot_atomic(&path, &SnapshotMeta::synthetic(), nodes, edges).unwrap_err();
        assert!(error.to_string().contains("unknotting-route invariant"));
        assert!(!path.exists());
    }

    #[test]
    fn full_integrity_pass_detects_program_corruption() {
        let path = test_path("program-hash");
        write_snapshot_atomic(
            &path,
            &SnapshotMeta::synthetic(),
            synthetic_nodes(2),
            synthetic_edges(2),
        )
        .unwrap();
        SqliteSnapshot::open_file(&path)
            .unwrap()
            .verify_program_hashes()
            .unwrap();

        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE programs SET program=?1 WHERE program_id=0",
            [b"corrupt"],
        )
        .unwrap();
        drop(conn);
        let error = SqliteSnapshot::open_file(&path)
            .unwrap()
            .verify_program_hashes()
            .unwrap_err();
        assert!(
            error.to_string().contains("hash mismatch"),
            "unexpected error: {error}"
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn schema_four_splits_keys_and_deduplicates_anchored_programs() {
        let path = test_path("program-dictionary");
        let nodes: Vec<_> = synthetic_nodes(3).collect();
        let mut edges: Vec<_> = synthetic_edges(3).collect();
        edges[1].program = edges[0].program.clone();
        write_snapshot_atomic(&path, &SnapshotMeta::synthetic(), nodes, edges).unwrap();

        let conn = Connection::open(&path).unwrap();
        let programs: i64 = conn
            .query_row("SELECT count(*) FROM programs", [], |row| row.get(0))
            .unwrap();
        let edges: i64 = conn
            .query_row("SELECT count(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        let edge_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(edges)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let node_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(nodes)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let node_keys: i64 = conn
            .query_row("SELECT count(*) FROM node_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(programs, 1);
        assert_eq!(edges, 2);
        assert!(edge_columns.contains(&"program_id".into()));
        assert!(edge_columns.contains(&"program_anchor_x".into()));
        assert!(edge_columns.contains(&"program_anchor_y".into()));
        assert!(!edge_columns.contains(&"program".into()));
        assert!(!edge_columns.contains(&"program_sha256".into()));
        assert!(!edge_columns.contains(&"validator_version".into()));
        assert_eq!(node_keys, 3);
        assert!(!node_columns.contains(&"rep_key".into()));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn production_snapshot_accepts_only_normalized_keyed_representations() {
        let path = test_path("production-normalization");
        let raw = representation::BraidRepresentation {
            strands: 3,
            cyclic_band_generators: false,
            word: vec![2, 1],
        };
        let normalized = raw.normalize().unwrap();
        assert_eq!(normalized.representation.word, vec![1, 2]);

        let make_node = |encoding: Vec<u8>| NodeRecord {
            rep_key: normalized.key,
            node_id: 0,
            role_mask: ROLE_CORE,
            encoding,
            u_upper_bound: None,
            unknot_route_rank: None,
            acs10: None,
            acs_route_rank: None,
            next_unknot: None,
            next_acs10: None,
            policy_stop: Some(cc_stop(0)),
        };
        let error = write_snapshot_atomic(
            &path,
            &production_meta(),
            [make_node(raw.encode().unwrap())],
            std::iter::empty(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("is not normalized"));

        write_snapshot_atomic(
            &path,
            &production_meta(),
            [make_node(normalized.encoded)],
            std::iter::empty(),
        )
        .unwrap();
        let graph = GraphSnapshot::load_file(&path).unwrap();
        assert_eq!(graph.lookup(&normalized.key).unwrap().node_id, 0);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn production_snapshot_allows_unattested_checkpoint_but_validates_policy_stop() {
        let path = test_path("production-policy-stop");
        let normalized = representation::BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        }
        .normalize()
        .unwrap();
        let make_node = |policy_stop| NodeRecord {
            rep_key: normalized.key,
            node_id: 0,
            role_mask: ROLE_CORE,
            encoding: normalized.encoded.clone(),
            u_upper_bound: None,
            unknot_route_rank: None,
            acs10: None,
            acs_route_rank: None,
            next_unknot: None,
            next_acs10: None,
            policy_stop,
        };

        write_snapshot_atomic(
            &path,
            &production_meta(),
            [make_node(None)],
            std::iter::empty(),
        )
        .unwrap();
        fs::remove_file(&path).unwrap();

        let not_cc = PolicyStopAttestation::PreferredCrossingChange {
            action: representation::SemanticAction::Reduce { position: 0 }
                .encode_u63()
                .unwrap(),
            audit_sha256: [10; 32],
        };
        let error = write_snapshot_atomic(
            &path,
            &production_meta(),
            [make_node(Some(not_cc))],
            std::iter::empty(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("is not CC"));
        assert!(!path.exists());
    }

    #[test]
    fn production_snapshot_rejects_multi_component_closures() {
        let path = test_path("production-knot-closure");
        let link = representation::BraidRepresentation {
            strands: 3,
            cyclic_band_generators: false,
            word: vec![1],
        }
        .normalize()
        .unwrap();
        let node = NodeRecord {
            rep_key: link.key,
            node_id: 0,
            role_mask: ROLE_CORE,
            encoding: link.encoded,
            u_upper_bound: None,
            unknot_route_rank: None,
            acs10: None,
            acs_route_rank: None,
            next_unknot: None,
            next_acs10: None,
            policy_stop: Some(cc_stop(0)),
        };
        let error = write_snapshot_atomic(&path, &production_meta(), [node], std::iter::empty())
            .unwrap_err();
        assert!(error.to_string().contains("does not close to one knot"));
        assert!(!path.exists());
    }

    #[test]
    fn production_writer_independently_replays_edge_programs() {
        let path = test_path("production-edge");
        let target = representation::BraidRepresentation {
            strands: 1,
            cyclic_band_generators: false,
            word: vec![],
        }
        .normalize()
        .unwrap();
        let source = representation::BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        }
        .normalize()
        .unwrap();
        let first = representation::SemanticAction::CrossingChange { position: 0 };
        let program = representation::ProofProgram {
            actions: vec![
                first,
                representation::SemanticAction::Reduce { position: 0 },
                representation::SemanticAction::Destabilize,
            ],
        };
        let route = RouteStep {
            edge_id: 0,
            target_node: 0,
            cc_cost: 1,
            first_action: first.encode_u63().unwrap(),
        };
        let nodes = vec![
            NodeRecord {
                rep_key: target.key,
                node_id: 0,
                role_mask: ROLE_CORE,
                encoding: target.encoded,
                u_upper_bound: Some(0),
                unknot_route_rank: Some(0),
                acs10: Some(10),
                acs_route_rank: Some(0),
                next_unknot: None,
                next_acs10: None,
                policy_stop: Some(PolicyStopAttestation::Terminal {
                    audit_sha256: [8; 32],
                }),
            },
            NodeRecord {
                rep_key: source.key,
                node_id: 1,
                role_mask: ROLE_CORE,
                encoding: source.encoded,
                u_upper_bound: Some(1),
                unknot_route_rank: Some(1),
                acs10: Some(28),
                acs_route_rank: Some(1),
                next_unknot: Some(route),
                next_acs10: Some(route),
                policy_stop: Some(PolicyStopAttestation::PreferredCrossingChange {
                    action: first.encode_u63().unwrap(),
                    audit_sha256: [9; 32],
                }),
            },
        ];
        let edge = EdgeRecord {
            edge_id: 0,
            source_node: 1,
            target_node: 0,
            cc_cost: 1,
            first_action: first.encode_u63().unwrap(),
            program_version: 0,
            program: program.encode().unwrap(),
            certificate_id: None,
            certificate: None,
            validator_version: representation::VALIDATOR_VERSION.into(),
        };
        write_snapshot_atomic(&path, &production_meta(), nodes.clone(), [edge.clone()]).unwrap();
        assert_eq!(
            GraphSnapshot::load_file(&path)
                .unwrap()
                .lookup(&source.key)
                .unwrap()
                .next_unknot,
            Some(route)
        );

        let bad_path = test_path("production-edge-bad");
        let mut bad_edge = edge;
        bad_edge.program = representation::ProofProgram {
            actions: vec![first],
        }
        .encode()
        .unwrap();
        let error =
            write_snapshot_atomic(&bad_path, &production_meta(), nodes, [bad_edge]).unwrap_err();
        assert!(error.to_string().contains("endpoint does not match"));
        assert!(!bad_path.exists());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_acs_route_that_does_not_decrease_rank() {
        let path = test_path("bad-rank");
        let mut nodes: Vec<_> = synthetic_nodes(2).collect();
        nodes[1].acs_route_rank = Some(0);
        let error =
            write_snapshot_atomic(&path, &SnapshotMeta::synthetic(), nodes, synthetic_edges(2))
                .unwrap_err();
        assert!(error.to_string().contains("ACS10-route invariant"));
        assert!(!path.exists());
    }

    #[test]
    fn rejects_non_dense_node_ids() {
        let path = test_path("sparse");
        let mut nodes: Vec<_> = synthetic_nodes(2).collect();
        nodes[1].node_id = 2;
        nodes[1].next_unknot = None;
        nodes[1].next_acs10 = None;
        let error =
            write_snapshot_atomic(&path, &SnapshotMeta::synthetic(), nodes, std::iter::empty())
                .unwrap_err();
        assert!(
            error.to_string().contains("must be dense"),
            "unexpected error: {error}"
        );
        assert!(!path.exists());
    }
}
