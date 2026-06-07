use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

use polo_core::{
    branch::BranchInfo,
    clock::Hlc,
    db::{RecordParams, RecordResult, RetractParams, ScanQuery},
    error::{Error, StorageError},
    fact::{Attr, BranchName, EntityId, Fact, FactId, Namespace, TxId, Value},
    merge::{ConflictEntry, ConflictResolution, DiffEntry, DiffParams, MergeParams, MergeResult},
    namespace::{MergePolicy, NamespaceInfo, NamespaceOpts},
    tx::Transaction,
    Store,
};

pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let conn = Connection::open(path).map_err(|e| storage_err(e.to_string()))?;
        Self::configure(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory().map_err(|e| storage_err(e.to_string()))?;
        Self::configure(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn configure(conn: &Connection) -> Result<(), Error> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| storage_err(e.to_string()))?;
        Self::run_migrations(conn)
    }

    fn run_migrations(conn: &Connection) -> Result<(), Error> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS namespaces (
                name           TEXT PRIMARY KEY,
                merge_policy   TEXT NOT NULL DEFAULT 'last_write_wins',
                schema_json    TEXT,
                created_at_ms  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS branches (
                namespace      TEXT NOT NULL REFERENCES namespaces(name),
                name           TEXT NOT NULL,
                parent         TEXT,
                fork_at        INTEGER,
                created_at_ms  INTEGER NOT NULL,
                head_tx        TEXT,
                closed         INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (namespace, name)
            );

            CREATE TABLE IF NOT EXISTS transactions (
                id             TEXT PRIMARY KEY,
                namespace      TEXT NOT NULL,
                branch         TEXT NOT NULL,
                ts             INTEGER NOT NULL,
                author         TEXT,
                message        TEXT,
                fact_count     INTEGER NOT NULL DEFAULT 0,
                caused_by      TEXT
            );

            CREATE TABLE IF NOT EXISTS facts (
                id             TEXT PRIMARY KEY,
                namespace      TEXT NOT NULL,
                entity         TEXT NOT NULL,
                attr           TEXT NOT NULL,
                value_type     TEXT NOT NULL,
                value_str      TEXT,
                value_int      INTEGER,
                value_float    REAL,
                value_bool     INTEGER,
                valid_from_ms  INTEGER NOT NULL,
                valid_to_ms    INTEGER,
                tx_id          TEXT NOT NULL,
                tx_time        INTEGER NOT NULL,
                branch         TEXT NOT NULL,
                author         TEXT,
                retracted      INTEGER NOT NULL DEFAULT 0,
                caused_by      TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_facts_lookup
                ON facts (namespace, branch, entity, attr, tx_time DESC);

            CREATE INDEX IF NOT EXISTS idx_facts_tx
                ON facts (tx_id);

            CREATE TABLE IF NOT EXISTS idempotency_cache (
                key            TEXT PRIMARY KEY,
                fact_id        TEXT NOT NULL,
                tx_id          TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| storage_err(e.to_string()))?;

        // Seed the default namespace and its main branch if not present.
        conn.execute(
            "INSERT OR IGNORE INTO namespaces (name, merge_policy, created_at_ms) VALUES (?1, ?2, ?3)",
            params!["default", "last_write_wins", now_ms()],
        )
        .map_err(|e| storage_err(e.to_string()))?;

        conn.execute(
            "INSERT OR IGNORE INTO branches (namespace, name, created_at_ms) VALUES (?1, ?2, ?3)",
            params!["default", "main", now_ms()],
        )
        .map_err(|e| storage_err(e.to_string()))?;

        Ok(())
    }

    fn with_conn<F, T>(&self, f: F) -> Result<T, Error>
    where
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error>,
    {
        let conn = self.conn.lock();
        f(&conn).map_err(|e| storage_err(e.to_string()))
    }
}

fn storage_err(msg: String) -> Error {
    Error::Storage(StorageError::Sqlite(msg))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn ms_to_dt(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

fn encode_value(v: &Value) -> (&'static str, Option<String>, Option<i64>, Option<f64>, Option<i32>) {
    match v {
        Value::Str(s) => ("str", Some(s.clone()), None, None, None),
        Value::Int(n) => ("int", None, Some(*n), None, None),
        Value::Float(f) => ("float", None, None, Some(*f), None),
        Value::Bool(b) => ("bool", None, None, None, Some(if *b { 1 } else { 0 })),
        Value::Json(j) => ("json", Some(j.to_string()), None, None, None),
        Value::Null => ("null", None, None, None, None),
    }
}

fn decode_value(
    vtype: &str,
    vstr: Option<String>,
    vint: Option<i64>,
    vfloat: Option<f64>,
    vbool: Option<i32>,
) -> Value {
    match vtype {
        "str" => Value::Str(vstr.unwrap_or_default()),
        "int" => Value::Int(vint.unwrap_or(0)),
        "float" => Value::Float(vfloat.unwrap_or(0.0)),
        "bool" => Value::Bool(vbool.unwrap_or(0) != 0),
        "json" => {
            let s = vstr.unwrap_or_default();
            serde_json::from_str(&s)
                .map(Value::Json)
                .unwrap_or(Value::Str(s))
        }
        _ => Value::Null,
    }
}

fn row_to_fact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Fact> {
    let id_str: String = row.get(0)?;
    let tx_id_str: String = row.get(11)?;
    let caused_by_str: Option<String> = row.get(16)?;

    let vtype: String = row.get(4)?;
    let vstr: Option<String> = row.get(5)?;
    let vint: Option<i64> = row.get(6)?;
    let vfloat: Option<f64> = row.get(7)?;
    let vbool: Option<i32> = row.get(8)?;

    let valid_from_ms: i64 = row.get(9)?;
    let valid_to_ms: Option<i64> = row.get(10)?;
    let tx_time_raw: i64 = row.get(12)?;

    Ok(Fact {
        id: FactId(id_str.parse().unwrap_or_default()),
        namespace: Namespace::new(row.get::<_, String>(1)?),
        entity: EntityId::new(row.get::<_, String>(2)?),
        attr: Attr::new(row.get::<_, String>(3)?),
        value: decode_value(&vtype, vstr, vint, vfloat, vbool),
        valid_from: ms_to_dt(valid_from_ms),
        valid_to: valid_to_ms.map(ms_to_dt),
        tx_id: TxId(tx_id_str.parse().unwrap_or_default()),
        tx_time: Hlc(tx_time_raw as u64),
        branch: BranchName::new(row.get::<_, String>(13)?),
        author: row.get(14)?,
        retracted: row.get::<_, i32>(15)? != 0,
        caused_by: caused_by_str
            .and_then(|s| s.parse().ok())
            .map(TxId),
    })
}

#[async_trait::async_trait]
impl Store for SqliteStore {
    async fn record(&self, p: RecordParams) -> Result<RecordResult, Error> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();

            // Check idempotency cache
            if let Some(key) = &p.idempotency_key {
                let existing: Option<(String, String)> = conn
                    .query_row(
                        "SELECT fact_id, tx_id FROM idempotency_cache WHERE key = ?1",
                        params![key],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(|e| storage_err(e.to_string()))?;

                if let Some((fid, tid)) = existing {
                    return Ok(RecordResult {
                        fact_id: FactId(fid.parse().unwrap_or_default()),
                        tx_id: TxId(tid.parse().unwrap_or_default()),
                        was_duplicate: true,
                    });
                }
            }

            let fact_id = FactId::new();
            let (vtype, vstr, vint, vfloat, vbool) = encode_value(&p.value);

            conn.execute(
                r#"INSERT INTO facts
                   (id, namespace, entity, attr, value_type, value_str, value_int, value_float,
                    value_bool, valid_from_ms, valid_to_ms, tx_id, tx_time, branch, author,
                    retracted, caused_by)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,0,?16)"#,
                params![
                    fact_id.to_string(),
                    p.namespace.as_str(),
                    p.entity.as_str(),
                    p.attr.as_str(),
                    vtype,
                    vstr,
                    vint,
                    vfloat,
                    vbool,
                    p.valid_from.timestamp_millis(),
                    p.valid_to.map(|t| t.timestamp_millis()),
                    p.tx_id.to_string(),
                    p.tx_time.0 as i64,
                    p.branch.as_str(),
                    p.author,
                    p.caused_by.map(|c| c.to_string()),
                ],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            // Upsert or insert transaction record
            conn.execute(
                r#"INSERT INTO transactions (id, namespace, branch, ts, author, message, fact_count, caused_by)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
                   ON CONFLICT(id) DO UPDATE SET fact_count = fact_count + 1"#,
                params![
                    p.tx_id.to_string(),
                    p.namespace.as_str(),
                    p.branch.as_str(),
                    p.tx_time.0 as i64,
                    p.author,
                    p.message,
                    p.caused_by.map(|c| c.to_string()),
                ],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            // Update branch head
            conn.execute(
                "UPDATE branches SET head_tx = ?1 WHERE namespace = ?2 AND name = ?3",
                params![p.tx_id.to_string(), p.namespace.as_str(), p.branch.as_str()],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            // Store idempotency key
            if let Some(key) = &p.idempotency_key {
                conn.execute(
                    "INSERT OR REPLACE INTO idempotency_cache (key, fact_id, tx_id) VALUES (?1,?2,?3)",
                    params![key, fact_id.to_string(), p.tx_id.to_string()],
                )
                .map_err(|e| storage_err(e.to_string()))?;
            }

            Ok(RecordResult {
                fact_id,
                tx_id: p.tx_id,
                was_duplicate: false,
            })
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn retract(&self, fact_id: FactId, p: RetractParams) -> Result<TxId, Error> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();

            let rows = conn
                .execute(
                    "UPDATE facts SET retracted = 1, valid_to_ms = ?1 WHERE id = ?2 AND namespace = ?3 AND branch = ?4 AND retracted = 0",
                    params![
                        p.tx_time.0 as i64,
                        fact_id.to_string(),
                        p.namespace.as_str(),
                        p.branch.as_str(),
                    ],
                )
                .map_err(|e| storage_err(e.to_string()))?;

            if rows == 0 {
                return Err(Error::FactNotFound(fact_id));
            }

            conn.execute(
                r#"INSERT INTO transactions (id, namespace, branch, ts, author, message, fact_count, caused_by)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, NULL)"#,
                params![
                    p.tx_id.to_string(),
                    p.namespace.as_str(),
                    p.branch.as_str(),
                    p.tx_time.0 as i64,
                    p.author,
                    p.message,
                ],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            conn.execute(
                "UPDATE branches SET head_tx = ?1 WHERE namespace = ?2 AND name = ?3",
                params![p.tx_id.to_string(), p.namespace.as_str(), p.branch.as_str()],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            Ok(p.tx_id)
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn get_fact(&self, id: FactId) -> Result<Fact, Error> {
        let conn = Arc::clone(&self.conn);
        let id_clone = id.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.query_row(
                "SELECT id,namespace,entity,attr,value_type,value_str,value_int,value_float,value_bool,
                         valid_from_ms,valid_to_ms,tx_id,tx_time,branch,author,retracted,caused_by
                  FROM facts WHERE id = ?1",
                params![id_clone.to_string()],
                row_to_fact,
            )
            .optional()
            .map_err(|e| storage_err(e.to_string()))?
            .ok_or(Error::FactNotFound(id))
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn scan(&self, q: ScanQuery) -> Result<Vec<Fact>, Error> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut sql = String::from(
                "SELECT id,namespace,entity,attr,value_type,value_str,value_int,value_float,value_bool,
                        valid_from_ms,valid_to_ms,tx_id,tx_time,branch,author,retracted,caused_by
                 FROM facts WHERE namespace = ?1 AND branch = ?2",
            );
            let mut bind_idx = 3;
            let mut extra_clauses = Vec::new();

            if q.entity.is_some() {
                extra_clauses.push(format!(" AND entity = ?{}", bind_idx));
                bind_idx += 1;
            }
            if q.attr.is_some() {
                extra_clauses.push(format!(" AND attr = ?{}", bind_idx));
                bind_idx += 1;
            }
            if !q.include_retracted {
                extra_clauses.push(" AND retracted = 0".into());
            }
            if let Some(asof) = q.asof_tx {
                extra_clauses.push(format!(" AND tx_time <= ?{}", bind_idx));
                bind_idx += 1;
                let _ = asof;
            }
            if let Some(eff) = q.asof_valid {
                extra_clauses.push(format!(
                    " AND valid_from_ms <= ?{b} AND (valid_to_ms IS NULL OR valid_to_ms > ?{b})",
                    b = bind_idx
                ));
                bind_idx += 1;
                let _ = eff;
            }

            for clause in &extra_clauses {
                sql.push_str(clause);
            }
            sql.push_str(" ORDER BY tx_time DESC");

            if let Some(lim) = q.limit {
                sql.push_str(&format!(" LIMIT {}", lim));
            }
            if let Some(off) = q.offset {
                sql.push_str(&format!(" OFFSET {}", off));
            }

            let mut stmt = conn.prepare(&sql).map_err(|e| storage_err(e.to_string()))?;

            // Dynamically bind positional parameters
            let ns = q.namespace.0.clone();
            let br = q.branch.0.clone();
            let ent = q.entity.as_ref().map(|e| e.0.clone());
            let attr = q.attr.as_ref().map(|a| a.0.clone());
            let asof_tx = q.asof_tx.map(|h| h.0 as i64);
            let asof_valid = q.asof_valid.map(|t| t.timestamp_millis());

            let mut values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(ns), Box::new(br)];
            if let Some(e) = ent {
                values.push(Box::new(e));
            }
            if let Some(a) = attr {
                values.push(Box::new(a));
            }
            if let Some(ts) = asof_tx {
                values.push(Box::new(ts));
            }
            if let Some(ms) = asof_valid {
                values.push(Box::new(ms));
            }

            let params = rusqlite::params_from_iter(values.iter().map(|v| v.as_ref()));
            let facts = stmt
                .query_map(params, row_to_fact)
                .map_err(|e| storage_err(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| storage_err(e.to_string()))?;

            Ok(facts)
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn get_branch(&self, ns: &Namespace, name: &BranchName) -> Result<BranchInfo, Error> {
        let conn = Arc::clone(&self.conn);
        let ns = ns.clone();
        let name = name.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.query_row(
                "SELECT namespace, name, parent, fork_at, created_at_ms, head_tx, closed
                 FROM branches WHERE namespace = ?1 AND name = ?2",
                params![ns.as_str(), name.as_str()],
                |row| {
                    let fork_at: Option<i64> = row.get(3)?;
                    let head_tx: Option<String> = row.get(5)?;
                    Ok(BranchInfo {
                        namespace: Namespace::new(row.get::<_, String>(0)?),
                        name: BranchName::new(row.get::<_, String>(1)?),
                        parent: row.get::<_, Option<String>>(2)?.map(BranchName::new),
                        fork_at: fork_at.map(|v| Hlc(v as u64)),
                        created_at: ms_to_dt(row.get(4)?),
                        head_tx: head_tx.and_then(|s| s.parse().ok()).map(TxId),
                        closed: row.get::<_, i32>(6)? != 0,
                    })
                },
            )
            .optional()
            .map_err(|e| storage_err(e.to_string()))?
            .ok_or(Error::BranchNotFound(name, ns))
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn create_branch(
        &self,
        ns: &Namespace,
        name: BranchName,
        fork_from: BranchName,
        fork_at: Hlc,
    ) -> Result<(), Error> {
        let conn = Arc::clone(&self.conn);
        let ns = ns.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();

            // Make sure namespace exists
            let ns_exists: bool = conn
                .query_row(
                    "SELECT 1 FROM namespaces WHERE name = ?1",
                    params![ns.as_str()],
                    |_| Ok(true),
                )
                .optional()
                .map_err(|e| storage_err(e.to_string()))?
                .is_some();

            if !ns_exists {
                return Err(Error::NamespaceNotFound(ns));
            }

            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM branches WHERE namespace = ?1 AND name = ?2",
                    params![ns.as_str(), name.as_str()],
                    |_| Ok(true),
                )
                .optional()
                .map_err(|e| storage_err(e.to_string()))?
                .is_some();

            if exists {
                return Err(Error::BranchExists(name, ns));
            }

            conn.execute(
                "INSERT INTO branches (namespace, name, parent, fork_at, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    ns.as_str(),
                    name.as_str(),
                    fork_from.as_str(),
                    fork_at.0 as i64,
                    now_ms(),
                ],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            Ok(())
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn list_branches(&self, ns: &Namespace) -> Result<Vec<BranchInfo>, Error> {
        let conn = Arc::clone(&self.conn);
        let ns = ns.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT namespace, name, parent, fork_at, created_at_ms, head_tx, closed
                     FROM branches WHERE namespace = ?1 ORDER BY created_at_ms ASC",
                )
                .map_err(|e| storage_err(e.to_string()))?;

            let branches = stmt
                .query_map(params![ns.as_str()], |row| {
                    let fork_at: Option<i64> = row.get(3)?;
                    let head_tx: Option<String> = row.get(5)?;
                    Ok(BranchInfo {
                        namespace: Namespace::new(row.get::<_, String>(0)?),
                        name: BranchName::new(row.get::<_, String>(1)?),
                        parent: row.get::<_, Option<String>>(2)?.map(BranchName::new),
                        fork_at: fork_at.map(|v| Hlc(v as u64)),
                        created_at: ms_to_dt(row.get(4)?),
                        head_tx: head_tx.and_then(|s| s.parse().ok()).map(TxId),
                        closed: row.get::<_, i32>(6)? != 0,
                    })
                })
                .map_err(|e| storage_err(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| storage_err(e.to_string()))?;

            Ok(branches)
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn delete_branch(&self, ns: &Namespace, name: &BranchName) -> Result<(), Error> {
        let conn = Arc::clone(&self.conn);
        let ns = ns.clone();
        let name = name.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();

            let info: Option<(Option<String>, i32)> = conn
                .query_row(
                    "SELECT parent, closed FROM branches WHERE namespace = ?1 AND name = ?2",
                    params![ns.as_str(), name.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|e| storage_err(e.to_string()))?;

            match info {
                None => return Err(Error::BranchNotFound(name, ns)),
                Some((None, _)) => return Err(Error::CannotDeleteRoot(name, ns)),
                Some(_) => {}
            }

            conn.execute(
                "DELETE FROM branches WHERE namespace = ?1 AND name = ?2",
                params![ns.as_str(), name.as_str()],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            Ok(())
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceInfo>, Error> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT name, merge_policy, schema_json, created_at_ms FROM namespaces ORDER BY name",
                )
                .map_err(|e| storage_err(e.to_string()))?;

            let ns_list = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|e| storage_err(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| storage_err(e.to_string()))?;

            ns_list
                .into_iter()
                .map(|(name, policy_str, schema_json, created_ms)| {
                    let merge_policy = policy_str.parse().unwrap_or_default();
                    let schema = schema_json
                        .map(|s| serde_json::from_str(&s))
                        .transpose()
                        .map_err(|e: serde_json::Error| {
                            Error::Storage(StorageError::Serde(e.to_string()))
                        })?;
                    Ok(NamespaceInfo {
                        name: Namespace::new(name),
                        merge_policy,
                        schema,
                        created_at: ms_to_dt(created_ms),
                    })
                })
                .collect()
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn get_namespace(&self, ns: &Namespace) -> Result<NamespaceInfo, Error> {
        let conn = Arc::clone(&self.conn);
        let ns = ns.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let row: Option<(String, Option<String>, i64)> = conn
                .query_row(
                    "SELECT merge_policy, schema_json, created_at_ms FROM namespaces WHERE name = ?1",
                    params![ns.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|e| storage_err(e.to_string()))?;

            match row {
                None => Err(Error::NamespaceNotFound(ns)),
                Some((policy_str, schema_json, created_ms)) => {
                    let merge_policy = policy_str.parse().unwrap_or_default();
                    let schema = schema_json
                        .map(|s| serde_json::from_str(&s))
                        .transpose()
                        .map_err(|e: serde_json::Error| {
                            Error::Storage(StorageError::Serde(e.to_string()))
                        })?;
                    Ok(NamespaceInfo {
                        name: ns,
                        merge_policy,
                        schema,
                        created_at: ms_to_dt(created_ms),
                    })
                }
            }
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn create_namespace(&self, ns: Namespace, opts: NamespaceOpts) -> Result<(), Error> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();

            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM namespaces WHERE name = ?1",
                    params![ns.as_str()],
                    |_| Ok(true),
                )
                .optional()
                .map_err(|e| storage_err(e.to_string()))?
                .is_some();

            if exists {
                return Err(Error::NamespaceExists(ns));
            }

            let schema_json = opts
                .schema
                .map(|s| serde_json::to_string(&s))
                .transpose()
                .map_err(|e: serde_json::Error| Error::Storage(StorageError::Serde(e.to_string())))?;

            conn.execute(
                "INSERT INTO namespaces (name, merge_policy, schema_json, created_at_ms) VALUES (?1,?2,?3,?4)",
                params![ns.as_str(), opts.merge_policy.to_string(), schema_json, now_ms()],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            // Create default main branch for this namespace
            conn.execute(
                "INSERT INTO branches (namespace, name, created_at_ms) VALUES (?1, 'main', ?2)",
                params![ns.as_str(), now_ms()],
            )
            .map_err(|e| storage_err(e.to_string()))?;

            Ok(())
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn get_tx(&self, id: &TxId) -> Result<Transaction, Error> {
        let conn = Arc::clone(&self.conn);
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.query_row(
                "SELECT id, namespace, branch, ts, author, message, fact_count, caused_by
                 FROM transactions WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    let caused_by: Option<String> = row.get(7)?;
                    Ok(Transaction {
                        id: TxId(row.get::<_, String>(0)?.parse().unwrap_or_default()),
                        namespace: Namespace::new(row.get::<_, String>(1)?),
                        branch: BranchName::new(row.get::<_, String>(2)?),
                        ts: Hlc(row.get::<_, i64>(3)? as u64),
                        author: row.get(4)?,
                        message: row.get(5)?,
                        fact_count: row.get::<_, i64>(6)? as usize,
                        caused_by: caused_by.and_then(|s| s.parse().ok()).map(TxId),
                    })
                },
            )
            .optional()
            .map_err(|e| storage_err(e.to_string()))?
            .ok_or(Error::TxNotFound(id))
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn list_tx(
        &self,
        ns: &Namespace,
        branch: &BranchName,
        limit: usize,
    ) -> Result<Vec<Transaction>, Error> {
        let conn = Arc::clone(&self.conn);
        let ns = ns.clone();
        let branch = branch.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT id, namespace, branch, ts, author, message, fact_count, caused_by
                     FROM transactions WHERE namespace = ?1 AND branch = ?2
                     ORDER BY ts DESC LIMIT ?3",
                )
                .map_err(|e| storage_err(e.to_string()))?;

            let txs = stmt
                .query_map(params![ns.as_str(), branch.as_str(), limit as i64], |row| {
                    let caused_by: Option<String> = row.get(7)?;
                    Ok(Transaction {
                        id: TxId(row.get::<_, String>(0)?.parse().unwrap_or_default()),
                        namespace: Namespace::new(row.get::<_, String>(1)?),
                        branch: BranchName::new(row.get::<_, String>(2)?),
                        ts: Hlc(row.get::<_, i64>(3)? as u64),
                        author: row.get(4)?,
                        message: row.get(5)?,
                        fact_count: row.get::<_, i64>(6)? as usize,
                        caused_by: caused_by.and_then(|s| s.parse().ok()).map(TxId),
                    })
                })
                .map_err(|e| storage_err(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| storage_err(e.to_string()))?;

            Ok(txs)
        })
        .await
        .map_err(|e| Error::Storage(StorageError::Join(e.to_string())))?
    }

    async fn merge(&self, p: MergeParams) -> Result<MergeResult, Error> {
        // Get all facts on source branch that aren't on target
        let source_facts = self
            .scan(ScanQuery {
                namespace: p.namespace.clone(),
                branch: p.source.clone(),
                ..Default::default()
            })
            .await?;

        let target_facts = self
            .scan(ScanQuery {
                namespace: p.namespace.clone(),
                branch: p.target.clone(),
                ..Default::default()
            })
            .await?;

        // Index target by (entity, attr) for conflict detection
        let mut target_idx: std::collections::HashMap<(String, String), Fact> =
            std::collections::HashMap::new();
        for f in target_facts {
            target_idx.insert((f.entity.0.clone(), f.attr.0.clone()), f);
        }

        let ns_info = self.get_namespace(&p.namespace).await?;
        let policy = ns_info.merge_policy;

        let tx_id = TxId::new();
        let ts = polo_core::clock::Clock::new().tick();
        let mut conflicts = Vec::new();
        let mut applied = 0;

        for src_fact in &source_facts {
            let key = (src_fact.entity.0.clone(), src_fact.attr.0.clone());
            if let Some(tgt_fact) = target_idx.get(&key) {
                // Both branches have a fact for this (entity, attr) — conflict
                match policy {
                    MergePolicy::ErrorOnConflict => {
                        return Err(Error::Conflict(format!(
                            "entity={} attr={} has different values on {} and {}",
                            src_fact.entity, src_fact.attr, p.source, p.target
                        )));
                    }
                    MergePolicy::FirstWriteWins => {
                        conflicts.push(ConflictEntry {
                            entity: src_fact.entity.to_string(),
                            attr: src_fact.attr.to_string(),
                            source_fact: src_fact.id.clone(),
                            target_fact: tgt_fact.id.clone(),
                            resolution: ConflictResolution::TargetWins,
                        });
                        continue;
                    }
                    MergePolicy::LastWriteWins => {
                        if src_fact.tx_time <= tgt_fact.tx_time {
                            conflicts.push(ConflictEntry {
                                entity: src_fact.entity.to_string(),
                                attr: src_fact.attr.to_string(),
                                source_fact: src_fact.id.clone(),
                                target_fact: tgt_fact.id.clone(),
                                resolution: ConflictResolution::TargetWins,
                            });
                            continue;
                        }
                        conflicts.push(ConflictEntry {
                            entity: src_fact.entity.to_string(),
                            attr: src_fact.attr.to_string(),
                            source_fact: src_fact.id.clone(),
                            target_fact: tgt_fact.id.clone(),
                            resolution: ConflictResolution::SourceWins,
                        });
                    }
                }
            }

            // Apply fact to target branch
            self.record(RecordParams {
                namespace: p.namespace.clone(),
                entity: src_fact.entity.clone(),
                attr: src_fact.attr.clone(),
                value: src_fact.value.clone(),
                branch: p.target.clone(),
                author: p.author.clone(),
                message: p.message.clone(),
                valid_from: src_fact.valid_from,
                valid_to: src_fact.valid_to,
                tx_time: ts,
                tx_id: tx_id.clone(),
                caused_by: p.caused_by.clone(),
                idempotency_key: None,
            })
            .await?;
            applied += 1;
        }

        Ok(MergeResult {
            tx_id,
            ts,
            facts_applied: applied,
            conflicts,
        })
    }

    async fn diff(&self, p: DiffParams) -> Result<Vec<DiffEntry>, Error> {
        let source_facts = self
            .scan(ScanQuery {
                namespace: p.namespace.clone(),
                branch: p.source.clone(),
                ..Default::default()
            })
            .await?;

        let target_facts = self
            .scan(ScanQuery {
                namespace: p.namespace.clone(),
                branch: p.target.clone(),
                ..Default::default()
            })
            .await?;

        let mut source_idx: std::collections::HashMap<(String, String), Fact> =
            std::collections::HashMap::new();
        for f in source_facts {
            source_idx.insert((f.entity.0.clone(), f.attr.0.clone()), f);
        }

        let mut target_idx: std::collections::HashMap<(String, String), Fact> =
            std::collections::HashMap::new();
        for f in target_facts {
            target_idx.insert((f.entity.0.clone(), f.attr.0.clone()), f);
        }

        let mut all_keys: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        all_keys.extend(source_idx.keys().cloned());
        all_keys.extend(target_idx.keys().cloned());

        let mut entries = Vec::new();
        for (entity, attr) in all_keys {
            let src = source_idx.get(&(entity.clone(), attr.clone())).cloned();
            let tgt = target_idx.get(&(entity.clone(), attr.clone())).cloned();
            if src != tgt {
                entries.push(DiffEntry {
                    entity,
                    attr,
                    source: src,
                    target: tgt,
                });
            }
        }

        entries.sort_by(|a, b| a.entity.cmp(&b.entity).then(a.attr.cmp(&b.attr)));
        Ok(entries)
    }

    async fn close(&self) {
        // rusqlite connection closes on drop; nothing explicit needed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polo_core::db::RecordOpts;

    async fn make_store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    #[tokio::test]
    async fn record_and_get() {
        let store = make_store().await;
        let ts = polo_core::clock::Clock::new().tick();
        let tx_id = TxId::new();
        let res = store
            .record(RecordParams {
                namespace: Namespace::default(),
                entity: EntityId::new("user/1"),
                attr: Attr::new("name"),
                value: Value::Str("alice".into()),
                branch: BranchName::main(),
                author: Some("test".into()),
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: ts,
                tx_id,
                caused_by: None,
                idempotency_key: None,
            })
            .await
            .unwrap();

        let fact = store.get_fact(res.fact_id).await.unwrap();
        assert_eq!(fact.entity.as_str(), "user/1");
        assert_eq!(fact.attr.as_str(), "name");
    }

    #[tokio::test]
    async fn idempotency() {
        let store = make_store().await;
        let ts = polo_core::clock::Clock::new().tick();
        let tx_id = TxId::new();
        let key = "idem-key-1".to_string();

        let r1 = store
            .record(RecordParams {
                namespace: Namespace::default(),
                entity: EntityId::new("e/1"),
                attr: Attr::new("v"),
                value: Value::Int(42),
                branch: BranchName::main(),
                author: None,
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: ts,
                tx_id: tx_id.clone(),
                caused_by: None,
                idempotency_key: Some(key.clone()),
            })
            .await
            .unwrap();

        let r2 = store
            .record(RecordParams {
                namespace: Namespace::default(),
                entity: EntityId::new("e/1"),
                attr: Attr::new("v"),
                value: Value::Int(99),
                branch: BranchName::main(),
                author: None,
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: polo_core::clock::Clock::new().tick(),
                tx_id: TxId::new(),
                caused_by: None,
                idempotency_key: Some(key),
            })
            .await
            .unwrap();

        assert!(r2.was_duplicate);
        assert_eq!(r1.fact_id, r2.fact_id);
    }
}
