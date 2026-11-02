//! Explicit, original-data benchmark. Never discovers or opens an application profile.
//! The same harness is copied into the pinned baseline checkout by benchmark-history.mjs.
use super::*;
use serde_json::{json, Value};
use std::{hint::black_box, path::PathBuf, time::Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "starclip-original-history-bench-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn samples<T>(iterations: usize, mut run: impl FnMut() -> T) -> (Value, T) {
    let mut times = Vec::with_capacity(iterations);
    let mut last = None;
    for _ in 0..iterations {
        let start = Instant::now();
        let result = black_box(run());
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        last = Some(result);
        times.push(elapsed);
    }
    let original = times.clone();
    times.sort_by(f64::total_cmp);
    let median = if iterations % 2 == 0 {
        (times[iterations / 2 - 1] + times[iterations / 2]) / 2.0
    } else {
        times[iterations / 2]
    };
    let p95 = times[(iterations * 95).div_ceil(100).saturating_sub(1)];
    (
        json!({"samplesMs":original,"medianMs":median,"p95Ms":p95,"iterations":iterations}),
        last.unwrap(),
    )
}
fn fingerprint(page: &HistoryPage) -> Value {
    let bytes =
        serde_json::to_vec(&json!({"clips": &page.clips, "history": &page.history})).unwrap();
    json!({"total":page.history.total,"page":page.history.page,"rows":page.clips.len(),
        "sha256":format!("{:x}",Sha256::digest(bytes))})
}
fn seed(path: &Path, rows: usize) -> (Store, Value) {
    let start = Instant::now();
    let db = Store::from_connection(Connection::open(path).unwrap()).unwrap();
    let tx = db.conn.unchecked_transaction().unwrap();
    for id in 1..=20 {
        tx.execute(
            "INSERT INTO groups(id,name,color) VALUES(?1,?2,'#778899')",
            params![id, format!("原创基准分组 {id}")],
        )
        .unwrap();
    }
    {
        let mut clip = tx.prepare("INSERT INTO clips(id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count,language,keep_forever) VALUES(?1,?2,?3,?4,?5,?6,?7,?7,?8,?9,?10)").unwrap();
        let mut privacy = tx
            .prepare("INSERT INTO clip_privacy(clip_id,metadata) VALUES(?1,?2)")
            .unwrap();
        let mut files = tx.prepare("INSERT INTO clip_files(clip_id,position,path,entry_kind) VALUES(?1,0,?2,'unknown')").unwrap();
        for id in 1..=rows {
            let kind = match id % 100 {
                0..=1 => "file",
                2..=9 => "link",
                10..=19 => "code",
                _ => "text",
            };
            let rare = if id % 10_000 == 37 {
                " rare-quasar"
            } else {
                ""
            };
            let short = if id % 13 == 0 { " 星河" } else { "" };
            let title = format!("原创基准 {id:06}{rare}{short}");
            let content = match kind {
                "file" => format!("/original-fixture/document-{id:06}.txt"),
                "link" => format!("https://fixture-{}.example/original/{id}",id%61),
                "code" => format!("fn original_fixture_{id}() {{ let orbit = {id}; }} // 固定原创代码{short}"),
                _ => format!("这是原创性能基准记录 {id:06}。用于比较搜索与分页，不来自真实剪贴板。The quiet orbit keeps an original note.{short}{rare}"),
            };
            let metadata = json!({"recordIdentity":format!("{id:064x}"),"privacyManual":id%19==0,
                "scanVersion":if id%37==0 {0}else{1},"manualTags":[format!("topic-{}",id%17),"原创基准"],
                "ruleTags":[format!("rule-{}",id%7)],"appTags":[format!("OriginalApp{}",id%5)],
                "linkHosts":if kind=="link" {vec![format!("fixture-{}.example",id%61)]}else{vec![]},
                "linkSources":if kind=="link" {vec![format!("source-{}",id%3)]}else{vec![]},
                "originHosts":[format!("fixture-{}.example",id%61)],"originSources":[format!("origin-{}",id%4)]});
            let group = (id % 5 == 0).then_some(((id / 5) % 20 + 1) as i64);
            clip.execute(params![
                id as i64,
                content,
                title,
                kind,
                id % 13 == 0,
                group,
                1_793_577_600_000_i64 + (id as i64 / 4),
                id % 9,
                if kind == "code" { Some("rust") } else { None },
                id % 17 == 0
            ])
            .unwrap();
            privacy
                .execute(params![id as i64, metadata.to_string()])
                .unwrap();
            if kind == "file" {
                files.execute(params![id as i64, content]).unwrap();
            }
        }
    }
    tx.commit().unwrap();
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    let sqlite: String = db
        .conn
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    let bytes = std::fs::metadata(path).unwrap().len();
    (
        db,
        json!({"rows":rows,"seedMs":elapsed,"databaseBytes":bytes,"sqliteVersion":sqlite,
        "shape":"80% text, 10% code, 8% links, 2% file references; 20 groups with exactly 20% of rows grouped; 17 manual, 7 rule, 5 app tags; 61 hosts; every 19th private, every 37th pending; fixed original strings and deterministic identities",
        "imageBlobs":false,"captureActivity":false}),
    )
}

// Diagnostic stages mirror the baseline SQL for these explicitly constructed cases.
// They run after full-query sampling, and never call the new global-statistics cache.
fn predicate(query: &HistoryQuery) -> (String, Vec<rusqlite::types::Value>) {
    let mut clauses = vec!["1=1".to_string()];
    let mut values = vec![];
    let search = query.query.trim().to_lowercase();
    if !search.is_empty() {
        if !query.ai_eligible {
            if let Some(phrase) = crate::search_index::match_phrase(&search) {
                clauses
                    .push("c.id IN (SELECT rowid FROM clips_fts WHERE clips_fts MATCH ?)".into());
                values.push(phrase.into());
            }
        }
        let tags = if query.ai_eligible {
            " || COALESCE(json_extract(p.metadata,'$.manualTags'),'') || COALESCE(json_extract(p.metadata,'$.ruleTags'),'') || COALESCE(json_extract(p.metadata,'$.appTags'),'')"
        } else {
            ""
        };
        clauses.push(format!("starclip_contains(c.title || char(10) || CASE WHEN c.kind='image' THEN '' ELSE c.content END{tags},?)"));
        values.push(search.into());
    }
    for source in &query.sources {
        let host = source
            .strip_prefix("host:")
            .expect("benchmark only constructs host filters");
        clauses.push("(EXISTS(SELECT 1 FROM json_each(p.metadata,'$.linkHosts') WHERE value=?) OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.originHosts') WHERE value=?))".into());
        values.extend([host.to_string().into(), host.to_string().into()]);
    }
    if !query.tag.is_empty() {
        clauses.push("(EXISTS(SELECT 1 FROM json_each(p.metadata,'$.manualTags') WHERE value=?) OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.ruleTags') WHERE value=?) OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.appTags') WHERE value=?))".into());
        values.extend([
            query.tag.clone().into(),
            query.tag.clone().into(),
            query.tag.clone().into(),
        ]);
    }
    if query.ai_eligible {
        clauses.push(format!("c.kind NOT IN ('image','file') AND NOT {PRIVATE_PREDICATE} AND NOT (c.kind='image' OR COALESCE(json_extract(p.metadata,'$.scanVersion'),0)!=1)"));
    }
    (clauses.join(" AND "), values)
}
fn raw_count(db: &Store, where_sql: &str, values: &[rusqlite::types::Value]) -> u64 {
    db.conn.query_row(&format!("SELECT COUNT(*) FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id WHERE {where_sql}"),rusqlite::params_from_iter(values.iter()),|r|r.get(0)).unwrap()
}
fn raw_page(
    db: &Store,
    query: &HistoryQuery,
    total: u64,
    where_sql: &str,
    values: &[rusqlite::types::Value],
) -> Vec<Clip> {
    let page = query
        .page
        .max(1)
        .min(total.div_ceil(u64::from(HISTORY_PAGE_SIZE)).max(1) as u32);
    let offset = u64::from(page - 1) * u64::from(HISTORY_PAGE_SIZE);
    let mut stmt=db.conn.prepare(&format!("{CLIP_SELECT} WHERE {where_sql} ORDER BY c.updated_at DESC,c.id DESC LIMIT {HISTORY_PAGE_SIZE} OFFSET {offset}")).unwrap();
    let mut clips = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), clip_from_row)
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    Store::fill_entries(&db.conn, &mut clips).unwrap();
    clips
}
fn raw_global_counts(db: &Store) -> Value {
    let mut counts = BTreeMap::<String, u64>::new();
    let mut stmt = db
        .conn
        .prepare("SELECT kind,COUNT(*) FROM clips GROUP BY kind")
        .unwrap();
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
        .unwrap()
    {
        let (key, value) = row.unwrap();
        counts.insert(key, value);
    }
    counts.insert("all".into(), counts.values().sum());
    for (key, condition) in [
        ("pinned", "c.pinned=1"),
        ("kept", "c.keep_forever=1"),
        (
            "link",
            "c.kind='link' OR json_array_length(p.metadata,'$.linkHosts')>0",
        ),
    ] {
        let value:u64=db.conn.query_row(&format!("SELECT COUNT(*) FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id WHERE {condition}"),[],|r|r.get(0)).unwrap();
        counts.insert(key.into(), value);
    }
    let mut stmt = db
        .conn
        .prepare("SELECT group_id,COUNT(*) FROM clips WHERE group_id IS NOT NULL GROUP BY group_id")
        .unwrap();
    let groups = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u64>(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .unwrap();
    json!({"counts":counts,"groupCounts":groups})
}
fn raw_facets(db: &Store) -> Value {
    let facet = |paths: &[&str]| {
        let sql = paths
            .iter()
            .map(|p| {
                format!(
                    "SELECT value FROM clip_privacy,json_each(metadata,'$.{p}') WHERE type='text'"
                )
            })
            .collect::<Vec<_>>()
            .join(" UNION ");
        let mut stmt = db
            .conn
            .prepare(&format!(
                "SELECT DISTINCT value FROM ({sql}) ORDER BY value"
            ))
            .unwrap();
        let result = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        result
    };
    json!({"sources":facet(&["linkSources","originSources"]),"hosts":facet(&["linkHosts","originHosts"]),"tags":facet(&["manualTags","ruleTags","appTags"])})
}

#[test]
#[ignore = "Explicit release-mode benchmark; creates only original disposable SQLite fixtures"]
fn original_history_queries() {
    assert!(
        !cfg!(debug_assertions),
        "Use --release for comparable performance samples"
    );
    let iterations: usize = std::env::var("STARCLIP_BENCH_ITERATIONS")
        .unwrap_or_else(|_| "20".into())
        .parse()
        .unwrap();
    assert!((2..=100).contains(&iterations));
    let sizes = std::env::var("STARCLIP_BENCH_SIZES")
        .unwrap_or_else(|_| "10000,50000,100000".into())
        .split(',')
        .map(|s| s.parse::<usize>().unwrap())
        .collect::<Vec<_>>();
    assert!(!sizes.is_empty() && sizes.iter().all(|n| (100..=100000).contains(n)));
    let mut datasets = vec![];
    for rows in sizes {
        let fixture = Fixture::new();
        let (db, seed) = seed(&fixture.0.join("original.sqlite3"), rows);
        let start = Instant::now();
        let first = db.history_page(&HistoryQuery::default()).unwrap();
        let first_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(first.history.total, rows as u64);
        let write = Instant::now();
        db.conn.execute("UPDATE clip_privacy SET metadata=json_set(metadata,'$.manualTags',json('[\"changed-only-tag\"]')) WHERE clip_id=?1",[rows as i64]).unwrap();
        let write_ms = write.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let after = db.history_page(&HistoryQuery::default()).unwrap();
        let after_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert!(after
            .history
            .tags
            .iter()
            .any(|tag| tag == "changed-only-tag"));
        let cases = vec![
            ("first_page", HistoryQuery::default()),
            (
                "rare_fts",
                HistoryQuery {
                    query: "rare-quasar".into(),
                    ..HistoryQuery::default()
                },
            ),
            (
                "short_two_characters",
                HistoryQuery {
                    query: "星河".into(),
                    ..HistoryQuery::default()
                },
            ),
            (
                "broad_fts",
                HistoryQuery {
                    query: "原创基准".into(),
                    ..HistoryQuery::default()
                },
            ),
            (
                "deep_page",
                HistoryQuery {
                    page: (rows as u32).div_ceil(HISTORY_PAGE_SIZE),
                    ..HistoryQuery::default()
                },
            ),
            (
                "tag_and_source",
                HistoryQuery {
                    tag: "topic-3".into(),
                    sources: vec!["host:fixture-7.example".into()],
                    ..HistoryQuery::default()
                },
            ),
            (
                "ai_tag_search",
                HistoryQuery {
                    query: "topic-5".into(),
                    ai_eligible: true,
                    ..HistoryQuery::default()
                },
            ),
        ];
        let mut results = vec![];
        // All end-to-end store queries precede diagnostics; no mutation occurs between warm samples.
        for (name, query) in &cases {
            let warmup = db.history_page(query).unwrap();
            let (timing, last) = samples(iterations, || db.history_page(query).unwrap());
            assert_eq!(fingerprint(&warmup), fingerprint(&last));
            results.push(json!({"name":name,"fullHistoryPage":timing,"result":fingerprint(&last)}));
        }
        for ((_, query), result) in cases.iter().zip(&mut results) {
            let (where_sql, values) = predicate(query);
            let (count_timing, total) = samples(iterations, || raw_count(&db, &where_sql, &values));
            assert_eq!(result["result"]["total"].as_u64().unwrap(), total);
            let (page_timing, page) = samples(iterations, || {
                raw_page(&db, query, total, &where_sql, &values)
            });
            assert_eq!(
                result["result"]["rows"].as_u64().unwrap(),
                page.len() as u64
            );
            result["uncachedDiagnosticStages"] =
                json!({"countMatching":count_timing,"readPage":page_timing});
        }
        let (counts_timing, counts) = samples(iterations, || raw_global_counts(&db));
        let (facets_timing, facets) = samples(iterations, || raw_facets(&db));
        datasets.push(json!({"fixture":seed,"coldStatisticsCache":{"firstAfterSeedMs":first_ms,"firstResult":fingerprint(&first),"facetMutationMs":write_ms,"firstAfterMutationMs":after_ms,"afterMutationResult":fingerprint(&after)},"warmQueries":results,"uncachedGlobalDiagnostics":{"counts":counts_timing,"facets":facets_timing,"result":json!({"counts":counts,"facets":facets})}}));
        drop(db);
        drop(fixture);
    }
    let output = json!({"format":"starclip-history-benchmark","version":1,"label":std::env::var("STARCLIP_BENCH_LABEL").unwrap_or_else(|_|"manual".into()),"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"iterations":iterations,"datasets":datasets,
        "scope":["actual Store::history_page only; no IPC, frontend rendering or Context mutex wait measurement","first/cold means empty or invalidated statistics cache, not cold OS/disk page cache; seeding warms SQLite/filesystem pages","full query samples precede independent raw-SQL diagnostics; diagnostics are not additive profiling of cached queries","all records/metadata are deterministic original fixtures; no clipboard or application data is read"]});
    let path = std::env::var("STARCLIP_BENCH_OUTPUT").expect("Set explicit artifact output path");
    std::fs::write(path, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    println!(
        "Original history benchmark completed: {} datasets × {} samples; no personal data",
        output["datasets"].as_array().unwrap().len(),
        iterations
    );
}
