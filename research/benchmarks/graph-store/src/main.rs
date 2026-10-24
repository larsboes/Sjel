//! Option A (libSQL edge table + recursive CTE) against option B (embedded Grafeo + Cypher)
//! on Graphify's graph of this repository. PLAN.md, "Integration candidates to narrow together".
//! A std BFS over the same edges is the reference answer for both.
//!
//! Usage: cargo run --release -- [graph.json] [seeds]

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

const CALLER_DEPTH: u32 = 3;
const PATH_DEPTH: u32 = 6;

struct Graph {
    keys: Vec<String>,
    index: HashMap<String, u32>,
    edges: Vec<(u32, u32, String)>,
}

fn load(path: &Path) -> Graph {
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("read graph.json")).expect("parse");
    let mut keys = Vec::new();
    let mut index = HashMap::new();
    for node in json["nodes"].as_array().expect("nodes") {
        let key = node["id"].as_str().expect("node id").to_owned();
        index.insert(key.clone(), keys.len() as u32);
        keys.push(key);
    }
    let edges = json["links"]
        .as_array()
        .expect("links")
        .iter()
        .filter_map(|l| {
            let s = *index.get(l["source"].as_str()?)?;
            let t = *index.get(l["target"].as_str()?)?;
            (s != t).then(|| (s, t, l["relation"].as_str().unwrap_or("related").to_owned()))
        })
        .collect();
    Graph { keys, index, edges }
}

/// Reference: every node with a `calls` path of 1..=depth hops into `target`.
fn ref_callers(callers_of: &[Vec<u32>], target: u32, depth: u32) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([(target, 0)]);
    while let Some((n, d)) = queue.pop_front() {
        if d == depth {
            continue;
        }
        for &c in &callers_of[n as usize] {
            if c != target && seen.insert(c) {
                queue.push_back((c, d + 1));
            }
        }
    }
    seen
}

/// Reference: undirected hop count over any relation, None beyond `depth`.
fn ref_distance(undirected: &[Vec<u32>], a: u32, b: u32, depth: u32) -> Option<u32> {
    let mut dist = HashMap::from([(a, 0)]);
    let mut queue = VecDeque::from([a]);
    while let Some(n) = queue.pop_front() {
        let d = dist[&n];
        if n == b {
            return Some(d);
        }
        if d == depth {
            continue;
        }
        for &m in &undirected[n as usize] {
            dist.entry(m).or_insert_with(|| {
                queue.push_back(m);
                d + 1
            });
        }
    }
    None
}

struct Timings(Vec<f64>);
impl Timings {
    fn report(mut self) -> String {
        self.0.sort_by(f64::total_cmp);
        let at = |q: f64| self.0[((self.0.len() - 1) as f64 * q) as usize];
        format!("p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms", at(0.5), at(0.95), at(1.0))
    }
}

fn dir_bytes(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|it| {
            it.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_bytes(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

async fn libsql_side(g: &Graph, dir: &Path, targets: &[u32], pairs: &[(u32, u32)]) -> Answers {
    let file = dir.join("a.db");
    let t = Instant::now();
    let db = libsql::Builder::new_local(&file).build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE nodes (id INTEGER PRIMARY KEY, key TEXT NOT NULL UNIQUE);
         CREATE TABLE edges (src INTEGER NOT NULL, dst INTEGER NOT NULL, rel TEXT NOT NULL);
         BEGIN;",
    )
    .await
    .unwrap();
    let ins = conn.prepare("INSERT INTO nodes VALUES (?1, ?2)").await.unwrap();
    for (i, k) in g.keys.iter().enumerate() {
        ins.execute((i as i64, k.as_str())).await.unwrap();
        ins.reset();
    }
    let ins = conn.prepare("INSERT INTO edges VALUES (?1, ?2, ?3)").await.unwrap();
    for (s, d, r) in &g.edges {
        ins.execute((*s as i64, *d as i64, r.as_str())).await.unwrap();
        ins.reset();
    }
    conn.execute_batch(
        "COMMIT;
         CREATE INDEX edges_dst_rel ON edges (dst, rel);
         CREATE INDEX edges_src ON edges (src);",
    )
    .await
    .unwrap();
    let load_ms = ms(t);
    drop(conn);
    drop(db);

    let t = Instant::now();
    let db = libsql::Builder::new_local(&file).build().await.unwrap();
    let conn = db.connect().unwrap();
    let reopen_ms = ms(t);

    // UNION on (n, d) keeps the recursion finite; DISTINCT n collapses the depths afterwards.
    let callers_sql = "WITH RECURSIVE r(n, d) AS (
            SELECT ?1, 0
            UNION SELECT e.src, r.d + 1 FROM edges e JOIN r ON e.dst = r.n
            WHERE e.rel = 'calls' AND r.d < ?2)
        SELECT DISTINCT n FROM r WHERE n <> ?1";
    let mut callers = Vec::new();
    let mut caller_t = Vec::new();
    for &target in targets {
        let t = Instant::now();
        let mut rows = conn.query(callers_sql, (target as i64, CALLER_DEPTH as i64)).await.unwrap();
        let mut set = BTreeSet::new();
        while let Some(row) = rows.next().await.unwrap() {
            set.insert(row.get::<i64>(0).unwrap() as u32);
        }
        caller_t.push(ms(t));
        callers.push(set);
    }

    // Bounded BFS in SQL: no visited set, so a node is re-expanded once per depth it is reached at.
    let path_sql = "WITH RECURSIVE r(n, d) AS (
            SELECT ?1, 0
            UNION SELECT CASE WHEN e.src = r.n THEN e.dst ELSE e.src END, r.d + 1
            FROM r JOIN edges e ON e.src = r.n OR e.dst = r.n
            WHERE r.d < ?3)
        SELECT min(d) FROM r WHERE n = ?2";
    let mut distances = Vec::new();
    let mut path_t = Vec::new();
    for &(a, b) in pairs {
        let t = Instant::now();
        let mut rows = conn.query(path_sql, (a as i64, b as i64, PATH_DEPTH as i64)).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        distances.push(row.get::<Option<i64>>(0).unwrap().map(|d| d as u32));
        path_t.push(ms(t));
    }
    Answers {
        load_ms,
        reopen_ms,
        bytes: dir_bytes(dir),
        callers,
        caller_t: Timings(caller_t),
        distances,
        path_t: Timings(path_t),
    }
}

fn grafeo_side(g: &Graph, dir: &Path, targets: &[u32], pairs: &[(u32, u32)]) -> Answers {
    use grafeo::{GrafeoDB, Value};
    let file = dir.join("b.grafeo");
    let t = Instant::now();
    let db = GrafeoDB::open(&file).unwrap();
    db.create_property_index("key").unwrap();
    let ids: Vec<_> = g
        .keys
        .iter()
        .map(|k| db.create_node_with_props(&["N"], [("key", Value::from(k.as_str()))]).unwrap())
        .collect();
    for (s, d, r) in &g.edges {
        db.create_edge(ids[*s as usize], ids[*d as usize], r).unwrap();
    }
    db.close().unwrap();
    let load_ms = ms(t);

    let t = Instant::now();
    let db = GrafeoDB::open(&file).unwrap();
    let reopen_ms = ms(t);

    let param = |pairs: &[(&str, u32)]| -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(name, n)| (name.to_string(), Value::from(g.keys[*n as usize].as_str())))
            .collect()
    };
    let text = |v: &Value| match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected a string, got {other:?}"),
    };

    let callers_q =
        format!("MATCH (t:N {{key: $k}})<-[:calls*1..{CALLER_DEPTH}]-(c:N) RETURN DISTINCT c.key");
    let mut callers = Vec::new();
    let mut caller_t = Vec::new();
    for &target in targets {
        let t = Instant::now();
        let res = db.execute_cypher_with_params(&callers_q, param(&[("k", target)])).unwrap();
        let set: BTreeSet<u32> = res
            .rows()
            .iter()
            .map(|row| g.index[&text(&row[0])])
            .filter(|&n| n != target)
            .collect();
        caller_t.push(ms(t));
        callers.push(set);
    }

    let path_q = format!(
        "MATCH p = shortestPath((a:N {{key: $a}})-[*..{PATH_DEPTH}]-(b:N {{key: $b}})) RETURN length(p)"
    );
    let mut distances = Vec::new();
    let mut path_t = Vec::new();
    for &(a, b) in pairs {
        let t = Instant::now();
        let res = db.execute_cypher_with_params(&path_q, param(&[("a", a), ("b", b)])).unwrap();
        distances.push(res.rows().first().map(|row| match &row[0] {
            Value::Int64(n) => *n as u32,
            other => panic!("expected a length, got {other:?}"),
        }));
        path_t.push(ms(t));
    }
    db.close().unwrap();
    Answers {
        load_ms,
        reopen_ms,
        bytes: dir_bytes(dir),
        callers,
        caller_t: Timings(caller_t),
        distances,
        path_t: Timings(path_t),
    }
}

struct Answers {
    load_ms: f64,
    reopen_ms: f64,
    bytes: u64,
    callers: Vec<BTreeSet<u32>>,
    caller_t: Timings,
    distances: Vec<Option<u32>>,
    path_t: Timings,
}

fn report(name: &str, a: Answers, ref_callers: &[BTreeSet<u32>], ref_dist: &[Option<u32>]) {
    let caller_wrong = a.callers.iter().zip(ref_callers).filter(|(x, y)| x != y).count();
    let dist_wrong = a.distances.iter().zip(ref_dist).filter(|(x, y)| x != y).count();
    println!("{name}");
    println!("  load+index {:.0} ms, reopen {:.1} ms, on disk {:.1} MB", a.load_ms, a.reopen_ms, a.bytes as f64 / 1e6);
    println!("  callers <= {CALLER_DEPTH} hops: {} / {} wrong, {}", caller_wrong, ref_callers.len(), a.caller_t.report());
    println!("  distance <= {PATH_DEPTH} hops: {} / {} wrong, {}", dist_wrong, ref_dist.len(), a.path_t.report());
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let graph_path = args.get(1).map(PathBuf::from).unwrap_or_else(|| "../../../graphify-out/graph.json".into());
    let seeds: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let g = load(&graph_path);
    let mut callers_of = vec![Vec::new(); g.keys.len()];
    let mut undirected = vec![Vec::new(); g.keys.len()];
    for (s, d, r) in &g.edges {
        if r == "calls" {
            callers_of[*d as usize].push(*s);
        }
        undirected[*s as usize].push(*d);
        undirected[*d as usize].push(*s);
    }
    println!("graph: {} nodes, {} edges, {} calls edges", g.keys.len(), g.edges.len(), callers_of.iter().map(Vec::len).sum::<usize>());

    // Deterministic seeds: every k-th called node in key order.
    let mut called: Vec<u32> = (0..g.keys.len() as u32).filter(|&n| !callers_of[n as usize].is_empty()).collect();
    called.sort_by(|a, b| g.keys[*a as usize].cmp(&g.keys[*b as usize]));
    let step = (called.len() / seeds).max(1);
    let targets: Vec<u32> = called.iter().step_by(step).take(seeds).copied().collect();
    let half = targets.len() / 2;
    let pairs: Vec<(u32, u32)> = (0..targets.len()).map(|i| (targets[i], targets[(i + half) % targets.len()])).collect();

    // A': the same edges held as in-memory adjacency, the shape SQL storage plus app-side BFS takes.
    let mut caller_t = Vec::new();
    let rc: Vec<_> = targets
        .iter()
        .map(|&t| {
            let start = Instant::now();
            let set = ref_callers(&callers_of, t, CALLER_DEPTH);
            caller_t.push(ms(start));
            set
        })
        .collect();
    let mut path_t = Vec::new();
    let rd: Vec<_> = pairs
        .iter()
        .map(|&(a, b)| {
            let start = Instant::now();
            let d = ref_distance(&undirected, a, b, PATH_DEPTH);
            path_t.push(ms(start));
            d
        })
        .collect();
    println!("A' in-memory adjacency (reference): callers {}; distance {}", Timings(caller_t).report(), Timings(path_t).report());
    let sizes: Vec<usize> = rc.iter().map(BTreeSet::len).collect();
    println!(
        "reference: caller sets sized {}..{} (sum {}), {} of {} pairs within {PATH_DEPTH} hops",
        sizes.iter().min().unwrap(), sizes.iter().max().unwrap(), sizes.iter().sum::<usize>(),
        rd.iter().filter(|d| d.is_some()).count(), rd.len()
    );

    let tmp = std::env::temp_dir().join(format!("graph-store-spike-{}", std::process::id()));
    for side in ["a", "b"] {
        std::fs::create_dir_all(tmp.join(side)).unwrap();
    }
    report("A libSQL 0.10.0, edge table + WITH RECURSIVE", libsql_side(&g, &tmp.join("a"), &targets, &pairs).await, &rc, &rd);
    report("B Grafeo 0.6.0, Cypher", grafeo_side(&g, &tmp.join("b"), &targets, &pairs), &rc, &rd);
    std::fs::remove_dir_all(&tmp).unwrap();
}
