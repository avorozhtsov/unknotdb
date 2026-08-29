use crate::representation::{CheckpointedProofProgram, ProofInstruction, ProofProgram};
use crate::Result;
use rusqlite::{params, Connection, OpenFlags};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs;
use std::path::{Path, PathBuf};

const INF: u64 = u64::MAX / 4;
const L1000_RATIO: u64 = 1000;

#[derive(Clone, Copy, Debug)]
struct EdgeCost {
    edge_id: u32,
    source: u32,
    target: u32,
    cc: u32,
    semantic: u64,
}

#[derive(Clone, Copy, Debug)]
struct FrontierRow {
    node: u32,
    cc: u32,
    delta_cc: u32,
    semantic: u64,
    next_edge: Option<u32>,
    next_target: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct RoutingSidecarReport {
    pub nodes: usize,
    pub edges: usize,
    pub terminals: usize,
    pub frontier_rows: usize,
    pub shortest_next_differs_from_active: usize,
    pub shortest_improves_active: usize,
    pub aggregate_semantic_saving: u64,
    pub maximum_semantic_saving: u64,
    pub l1000_next_differs_from_shortest: usize,
    pub l1000_uses_more_cc: usize,
    pub bytes: u64,
    pub proof_sha256: String,
}

fn active_semantic_lengths(
    active_edge: &[Option<u32>],
    edges: &[EdgeCost],
    graph_u: &[Option<u32>],
) -> Result<Vec<u64>> {
    let mut by_id = vec![None; edges.len()];
    for &edge in edges {
        if edge.edge_id as usize >= by_id.len() {
            return Err("proof edge IDs are not dense".into());
        }
        by_id[edge.edge_id as usize] = Some(edge);
    }
    fn visit(
        node: usize,
        active_edge: &[Option<u32>],
        by_id: &[Option<EdgeCost>],
        graph_u: &[Option<u32>],
        state: &mut [u8],
        distance: &mut [u64],
    ) -> Result<u64> {
        if state[node] == 2 {
            return Ok(distance[node]);
        }
        if state[node] == 1 {
            return Err(format!("active unknot route contains a cycle at node {node}").into());
        }
        state[node] = 1;
        let value = if let Some(edge_id) = active_edge[node] {
            let edge = by_id
                .get(edge_id as usize)
                .and_then(|edge| *edge)
                .ok_or("active unknot route references an absent edge")?;
            if edge.source as usize != node {
                return Err("active unknot edge has the wrong source".into());
            }
            edge.semantic.saturating_add(visit(
                edge.target as usize,
                active_edge,
                by_id,
                graph_u,
                state,
                distance,
            )?)
        } else if graph_u[node] == Some(0) {
            0
        } else {
            return Err(format!("nonterminal node {node} has no active unknot edge").into());
        };
        distance[node] = value;
        state[node] = 2;
        Ok(value)
    }

    let mut state = vec![0_u8; active_edge.len()];
    let mut distance = vec![0_u64; active_edge.len()];
    for node in 0..active_edge.len() {
        visit(
            node,
            active_edge,
            &by_id,
            graph_u,
            &mut state,
            &mut distance,
        )?;
    }
    Ok(distance)
}

fn semantic_len(version: u32, bytes: &[u8]) -> Result<u64> {
    let count = match version {
        0 => ProofProgram::decode(bytes)?.actions.len(),
        CheckpointedProofProgram::VERSION => CheckpointedProofProgram::decode(bytes)?
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction, ProofInstruction::Action(_)))
            .count(),
        _ => return Err(format!("unsupported proof program version {version}").into()),
    };
    count
        .try_into()
        .map_err(|_| "semantic program length exceeds u64".into())
}

type RouteArrays = (Vec<u32>, Vec<u64>, Vec<Option<u32>>, Vec<Option<u32>>);

fn lexicographic_routes(incoming: &[Vec<EdgeCost>], terminals: &[u32]) -> RouteArrays {
    let count = incoming.len();
    let mut cc = vec![u32::MAX; count];
    let mut semantic = vec![INF; count];
    let mut next_edge = vec![None; count];
    let mut next_target = vec![None; count];
    let mut heap = BinaryHeap::new();
    for &terminal in terminals {
        cc[terminal as usize] = 0;
        semantic[terminal as usize] = 0;
        heap.push(Reverse((0_u32, 0_u64, terminal)));
    }
    while let Some(Reverse((known_cc, known_semantic, target))) = heap.pop() {
        let target_index = target as usize;
        if (known_cc, known_semantic) != (cc[target_index], semantic[target_index]) {
            continue;
        }
        for &edge in &incoming[target_index] {
            let source = edge.source as usize;
            let candidate = (
                known_cc.saturating_add(edge.cc),
                known_semantic.saturating_add(edge.semantic),
            );
            let current = (cc[source], semantic[source]);
            if candidate < current {
                cc[source] = candidate.0;
                semantic[source] = candidate.1;
                next_edge[source] = Some(edge.edge_id);
                next_target[source] = Some(edge.target);
                heap.push(Reverse((candidate.0, candidate.1, edge.source)));
            } else if candidate == current && Some(edge.edge_id) < next_edge[source] {
                next_edge[source] = Some(edge.edge_id);
                next_target[source] = Some(edge.target);
            }
        }
    }
    (cc, semantic, next_edge, next_target)
}

type L1000Arrays = (
    Vec<u64>,
    Vec<u32>,
    Vec<u64>,
    Vec<Option<u32>>,
    Vec<Option<u32>>,
);

fn l1000_routes(incoming: &[Vec<EdgeCost>], terminals: &[u32]) -> L1000Arrays {
    let count = incoming.len();
    let mut score = vec![INF; count];
    let mut cc = vec![u32::MAX; count];
    let mut semantic = vec![INF; count];
    let mut next_edge = vec![None; count];
    let mut next_target = vec![None; count];
    let mut heap = BinaryHeap::new();
    for &terminal in terminals {
        let index = terminal as usize;
        score[index] = 0;
        cc[index] = 0;
        semantic[index] = 0;
        heap.push(Reverse((0_u64, 0_u32, 0_u64, terminal)));
    }
    while let Some(Reverse((known_score, known_cc, known_semantic, target))) = heap.pop() {
        let target_index = target as usize;
        if (known_score, known_cc, known_semantic)
            != (
                score[target_index],
                cc[target_index],
                semantic[target_index],
            )
        {
            continue;
        }
        for &edge in &incoming[target_index] {
            let source = edge.source as usize;
            let candidate_cc = known_cc.saturating_add(edge.cc);
            let candidate_semantic = known_semantic.saturating_add(edge.semantic);
            let candidate_score = known_score
                .saturating_add(L1000_RATIO.saturating_mul(u64::from(edge.cc)))
                .saturating_add(edge.semantic);
            let candidate = (candidate_score, candidate_cc, candidate_semantic);
            let current = (score[source], cc[source], semantic[source]);
            if candidate < current {
                score[source] = candidate_score;
                cc[source] = candidate_cc;
                semantic[source] = candidate_semantic;
                next_edge[source] = Some(edge.edge_id);
                next_target[source] = Some(edge.target);
                heap.push(Reverse((
                    candidate_score,
                    candidate_cc,
                    candidate_semantic,
                    edge.source,
                )));
            } else if candidate == current && Some(edge.edge_id) < next_edge[source] {
                next_edge[source] = Some(edge.edge_id);
                next_target[source] = Some(edge.target);
            }
        }
    }
    (score, cc, semantic, next_edge, next_target)
}

fn zero_cc_closure(
    incoming: &[Vec<EdgeCost>],
    semantic: &mut [u64],
    next_edge: &mut [Option<u32>],
    next_target: &mut [Option<u32>],
) {
    let mut heap = BinaryHeap::new();
    for (node, &distance) in semantic.iter().enumerate() {
        if distance < INF {
            heap.push(Reverse((distance, node as u32)));
        }
    }
    while let Some(Reverse((known, target))) = heap.pop() {
        if known != semantic[target as usize] {
            continue;
        }
        for &edge in incoming[target as usize].iter().filter(|edge| edge.cc == 0) {
            let source = edge.source as usize;
            let candidate = known.saturating_add(edge.semantic);
            if candidate < semantic[source] {
                semantic[source] = candidate;
                next_edge[source] = Some(edge.edge_id);
                next_target[source] = Some(edge.target);
                heap.push(Reverse((candidate, edge.source)));
            } else if candidate == semantic[source] && Some(edge.edge_id) < next_edge[source] {
                next_edge[source] = Some(edge.edge_id);
                next_target[source] = Some(edge.target);
            }
        }
    }
}

fn bounded_frontier(
    incoming: &[Vec<EdgeCost>],
    edges: &[EdgeCost],
    terminals: &[u32],
    graph_u: &[Option<u32>],
) -> Vec<FrontierRow> {
    let count = incoming.len();
    let max_cc = graph_u.iter().flatten().copied().max().unwrap_or(0) + 2;
    let mut previous = vec![INF; count];
    let mut rows = Vec::new();
    for &terminal in terminals {
        previous[terminal as usize] = 0;
    }
    for exact_cc in 0..=max_cc {
        let mut semantic;
        let mut next_edge = vec![None; count];
        let mut next_target = vec![None; count];
        if exact_cc == 0 {
            semantic = previous;
        } else {
            semantic = vec![INF; count];
            for &edge in edges.iter().filter(|edge| edge.cc == 1) {
                let tail = previous[edge.target as usize];
                if tail == INF {
                    continue;
                }
                let source = edge.source as usize;
                let candidate = tail.saturating_add(edge.semantic);
                if candidate < semantic[source]
                    || (candidate == semantic[source] && Some(edge.edge_id) < next_edge[source])
                {
                    semantic[source] = candidate;
                    next_edge[source] = Some(edge.edge_id);
                    next_target[source] = Some(edge.target);
                }
            }
        }
        zero_cc_closure(incoming, &mut semantic, &mut next_edge, &mut next_target);
        for node in 0..count {
            let Some(min_cc) = graph_u[node] else {
                continue;
            };
            if exact_cc >= min_cc && exact_cc <= min_cc + 2 && semantic[node] < INF {
                rows.push(FrontierRow {
                    node: node as u32,
                    cc: exact_cc,
                    delta_cc: exact_cc - min_cc,
                    semantic: semantic[node],
                    next_edge: next_edge[node],
                    next_target: next_target[node],
                });
            }
        }
        previous = semantic;
    }
    rows
}

fn temporary_path(output: &Path) -> Result<PathBuf> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("routing sidecar path has no UTF-8 file name")?;
    Ok(parent.join(format!(".{name}.{}.part", std::process::id())))
}

pub fn build_routing_sidecar(proof: &Path, output: &Path) -> Result<RoutingSidecarReport> {
    if output.exists() {
        return Err(format!("routing sidecar already exists: {}", output.display()).into());
    }
    let proof_bytes = fs::read(proof)?;
    let proof_sha256 = unknotdb::util::sha256_hex(&proof_bytes);
    let connection = Connection::open_with_flags(proof, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut graph_u = Vec::new();
    let mut active_edge = Vec::new();
    {
        let mut statement = connection
            .prepare("SELECT node_id,u_upper_bound,next_unknot_edge FROM nodes ORDER BY node_id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, Option<u32>>(1)?,
                row.get::<_, Option<u32>>(2)?,
            ))
        })?;
        for row in rows {
            let (node, u, edge) = row?;
            if node as usize != graph_u.len() {
                return Err("proof graph node IDs are not dense".into());
            }
            graph_u.push(u);
            active_edge.push(edge);
        }
    }
    let terminals: Vec<u32> = graph_u
        .iter()
        .enumerate()
        .filter_map(|(node, &u)| (u == Some(0)).then_some(node as u32))
        .collect();
    if terminals.is_empty() {
        return Err("proof graph has no terminal U=0 vertex".into());
    }
    let mut edges = Vec::new();
    let mut incoming = vec![Vec::new(); graph_u.len()];
    {
        let mut statement = connection.prepare(
            r#"
            SELECT e.edge_id,e.source_node,e.target_node,e.cc_cost,
                   p.program_version,p.program
            FROM edges e JOIN programs p USING(program_id)
            ORDER BY e.edge_id
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        })?;
        for row in rows {
            let (edge_id, source, target, cc, version, program) = row?;
            if source as usize >= graph_u.len() || target as usize >= graph_u.len() || cc > 1 {
                return Err("proof edge has an invalid routing endpoint or CC cost".into());
            }
            let edge = EdgeCost {
                edge_id,
                source,
                target,
                cc,
                semantic: semantic_len(version, &program)?,
            };
            incoming[target as usize].push(edge);
            edges.push(edge);
        }
    }
    let (min_cc, shortest_semantic, shortest_edge, shortest_target) =
        lexicographic_routes(&incoming, &terminals);
    for (node, expected) in graph_u.iter().enumerate() {
        match expected {
            Some(value) if min_cc[node] == *value => {}
            Some(value) => {
                return Err(format!(
                    "node {node} graph U {value} disagrees with lexicographic distance {}",
                    min_cc[node]
                )
                .into())
            }
            None if min_cc[node] == u32::MAX => {}
            None => return Err(format!("node {node} is routable but graph U is NULL").into()),
        }
    }
    let (l1000_score, l1000_cc, l1000_semantic, l1000_edge, l1000_target) =
        l1000_routes(&incoming, &terminals);
    let active_semantic = active_semantic_lengths(&active_edge, &edges, &graph_u)?;
    let frontier = bounded_frontier(&incoming, &edges, &terminals, &graph_u);
    for row in frontier.iter().filter(|row| row.delta_cc == 0) {
        if shortest_semantic[row.node as usize] != row.semantic {
            return Err(format!(
                "node {} exact-U profile disagrees with shortest route",
                row.node
            )
            .into());
        }
    }

    let shortest_next_differs_from_active = shortest_edge
        .iter()
        .zip(&active_edge)
        .filter(|(left, right)| left != right)
        .count();
    let semantic_savings: Vec<u64> = active_semantic
        .iter()
        .zip(&shortest_semantic)
        .map(|(&active, &shortest)| active.saturating_sub(shortest))
        .collect();
    let shortest_improves_active = semantic_savings
        .iter()
        .filter(|&&saving| saving > 0)
        .count();
    let aggregate_semantic_saving = semantic_savings.iter().sum();
    let maximum_semantic_saving = semantic_savings.iter().copied().max().unwrap_or(0);
    let l1000_next_differs_from_shortest = l1000_edge
        .iter()
        .zip(&shortest_edge)
        .filter(|(left, right)| left != right)
        .count();
    let l1000_uses_more_cc = l1000_cc
        .iter()
        .zip(&min_cc)
        .filter(|(left, right)| **left != u32::MAX && left > right)
        .count();

    let temporary = temporary_path(output)?;
    let mut sidecar = Connection::open(&temporary)?;
    sidecar.execute_batch(
        r#"
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        PRAGMA user_version=1;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE edge_costs(
            edge_id INTEGER PRIMARY KEY,
            cc_cost INTEGER NOT NULL CHECK(cc_cost BETWEEN 0 AND 1),
            semantic_moves INTEGER NOT NULL CHECK(semantic_moves > 0),
            l1000_cost INTEGER NOT NULL CHECK(l1000_cost > 0)
        );
        CREATE TABLE routes(
            node_id INTEGER PRIMARY KEY,
            min_cc INTEGER,
            shortest_at_min_cc_semantic_moves INTEGER,
            next_shortest_edge INTEGER,
            next_shortest_target INTEGER,
            l1000_score INTEGER,
            l1000_cc INTEGER,
            l1000_semantic_moves INTEGER,
            next_l1000_edge INTEGER,
            next_l1000_target INTEGER,
            graph_next_unknot_edge INTEGER,
            graph_active_semantic_moves INTEGER
        );
        CREATE INDEX routes_by_l1000 ON routes(l1000_score,node_id);
        CREATE TABLE route_frontier(
            node_id INTEGER NOT NULL,
            cc_count INTEGER NOT NULL,
            delta_from_min_cc INTEGER NOT NULL CHECK(delta_from_min_cc BETWEEN 0 AND 2),
            semantic_moves INTEGER NOT NULL,
            next_edge INTEGER,
            next_target INTEGER,
            PRIMARY KEY(node_id,cc_count)
        ) WITHOUT ROWID;
        CREATE INDEX frontier_by_budget ON route_frontier(cc_count,semantic_moves,node_id);
        "#,
    )?;
    let transaction = sidecar.transaction()?;
    transaction.execute(
        "INSERT INTO meta VALUES ('schema','unknotdb-routing-sidecar-v1')",
        [],
    )?;
    transaction.execute(
        "INSERT INTO meta VALUES ('proof_sha256',?1)",
        [&proof_sha256],
    )?;
    transaction.execute(
        "INSERT INTO meta VALUES ('semantic_length','count of semantic Action instructions; coordinate-only instructions cost zero')",
        [],
    )?;
    transaction.execute(
        "INSERT INTO meta VALUES ('l1000_objective','1000*CC + semantic_moves')",
        [],
    )?;
    transaction.execute(
        "INSERT INTO meta VALUES ('frontier_scope','exact CC counts U,U+1,U+2 when reachable')",
        [],
    )?;
    {
        let mut insert = transaction.prepare("INSERT INTO edge_costs VALUES (?1,?2,?3,?4)")?;
        for edge in &edges {
            insert.execute(params![
                edge.edge_id,
                edge.cc,
                i64::try_from(edge.semantic)?,
                i64::try_from(L1000_RATIO * u64::from(edge.cc) + edge.semantic)?,
            ])?;
        }
    }
    {
        let mut insert = transaction
            .prepare("INSERT INTO routes VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")?;
        for node in 0..graph_u.len() {
            let reachable = min_cc[node] != u32::MAX;
            insert.execute(params![
                node as u32,
                reachable.then_some(min_cc[node]),
                reachable
                    .then(|| i64::try_from(shortest_semantic[node]))
                    .transpose()?,
                shortest_edge[node],
                shortest_target[node],
                (l1000_score[node] != INF)
                    .then(|| i64::try_from(l1000_score[node]))
                    .transpose()?,
                (l1000_cc[node] != u32::MAX).then_some(l1000_cc[node]),
                (l1000_semantic[node] != INF)
                    .then(|| i64::try_from(l1000_semantic[node]))
                    .transpose()?,
                l1000_edge[node],
                l1000_target[node],
                active_edge[node],
                i64::try_from(active_semantic[node])?,
            ])?;
        }
    }
    {
        let mut insert =
            transaction.prepare("INSERT INTO route_frontier VALUES (?1,?2,?3,?4,?5,?6)")?;
        for row in &frontier {
            insert.execute(params![
                row.node,
                row.cc,
                row.delta_cc,
                i64::try_from(row.semantic)?,
                row.next_edge,
                row.next_target,
            ])?;
        }
    }
    transaction.commit()?;
    sidecar.execute_batch("PRAGMA optimize;")?;
    if sidecar.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))? != "ok" {
        return Err("routing sidecar integrity check failed".into());
    }
    drop(sidecar);
    fs::rename(&temporary, output)?;
    let bytes = fs::metadata(output)?.len();
    Ok(RoutingSidecarReport {
        nodes: graph_u.len(),
        edges: edges.len(),
        terminals: terminals.len(),
        frontier_rows: frontier.len(),
        shortest_next_differs_from_active,
        shortest_improves_active,
        aggregate_semantic_saving,
        maximum_semantic_saving,
        l1000_next_differs_from_shortest,
        l1000_uses_more_cc,
        bytes,
        proof_sha256,
    })
}

pub fn show_routing(proof: &Path, sidecar: &Path, identifier: &str) -> Result<String> {
    let proof_sha256 = unknotdb::util::sha256_hex(&fs::read(proof)?);
    let proof_connection = Connection::open_with_flags(proof, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let node = if let Ok(node) = identifier.parse::<u32>() {
        node
    } else {
        if identifier.len() != 64 || !identifier.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("routing identifier must be a node ID or 64-digit rep key".into());
        }
        let mut key = [0_u8; 32];
        for (index, pair) in identifier.as_bytes().chunks_exact(2).enumerate() {
            key[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
        }
        proof_connection.query_row(
            "SELECT node_id FROM node_keys WHERE rep_key=?1",
            [key.as_slice()],
            |row| row.get(0),
        )?
    };
    let connection = Connection::open_with_flags(sidecar, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let pinned: String = connection.query_row(
        "SELECT value FROM meta WHERE key='proof_sha256'",
        [],
        |row| row.get(0),
    )?;
    if pinned != proof_sha256 {
        return Err("routing sidecar is not pinned to this proof snapshot".into());
    }
    let route = connection.query_row(
        r#"
        SELECT min_cc,shortest_at_min_cc_semantic_moves,
               next_shortest_edge,next_shortest_target,
               l1000_score,l1000_cc,l1000_semantic_moves,
               next_l1000_edge,next_l1000_target,
               graph_next_unknot_edge,graph_active_semantic_moves
        FROM routes WHERE node_id=?1
        "#,
        [node],
        |row| {
            Ok((
                row.get::<_, Option<u32>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<u32>>(2)?,
                row.get::<_, Option<u32>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<u32>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<u32>>(7)?,
                row.get::<_, Option<u32>>(8)?,
                row.get::<_, Option<u32>>(9)?,
                row.get::<_, i64>(10)?,
            ))
        },
    )?;
    let mut output = format!(
        "node_id: {node}\nproof_sha256: {proof_sha256}\nmin_cc: {:?}\n\
shortest_at_min_cc_semantic_moves: {:?}\nnext_shortest_edge: {:?}\n\
next_shortest_target: {:?}\ngraph_active_semantic_moves: {}\n\
graph_next_unknot_edge: {:?}\nl1000_score: {:?}\nl1000_cc: {:?}\n\
l1000_semantic_moves: {:?}\nnext_l1000_edge: {:?}\nnext_l1000_target: {:?}\n",
        route.0,
        route.1,
        route.2,
        route.3,
        route.10,
        route.9,
        route.4,
        route.5,
        route.6,
        route.7,
        route.8,
    );
    output.push_str("frontier:\n");
    let mut statement = connection.prepare(
        "SELECT cc_count,delta_from_min_cc,semantic_moves,next_edge,next_target \
         FROM route_frontier WHERE node_id=?1 ORDER BY cc_count",
    )?;
    let rows = statement.query_map([node], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<u32>>(3)?,
            row.get::<_, Option<u32>>(4)?,
        ))
    })?;
    for row in rows {
        let (cc, delta, semantic, edge, target) = row?;
        output.push_str(&format!(
            "  cc={cc} delta={delta} semantic_moves={semantic} next_edge={edge:?} next_target={target:?}\n"
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1000_can_trade_one_more_cc_for_many_fewer_semantic_moves() {
        let edges = [
            EdgeCost {
                edge_id: 0,
                source: 1,
                target: 0,
                cc: 1,
                semantic: 1501,
            },
            EdgeCost {
                edge_id: 1,
                source: 1,
                target: 2,
                cc: 1,
                semantic: 1,
            },
            EdgeCost {
                edge_id: 2,
                source: 2,
                target: 0,
                cc: 1,
                semantic: 1,
            },
        ];
        let mut incoming = vec![Vec::new(); 3];
        for &edge in &edges {
            incoming[edge.target as usize].push(edge);
        }
        let (min_cc, semantic, next, _) = lexicographic_routes(&incoming, &[0]);
        assert_eq!((min_cc[1], semantic[1], next[1]), (1, 1501, Some(0)));
        let (score, cc, semantic, next, _) = l1000_routes(&incoming, &[0]);
        assert_eq!(
            (score[1], cc[1], semantic[1], next[1]),
            (2002, 2, 2, Some(1))
        );
    }
}
