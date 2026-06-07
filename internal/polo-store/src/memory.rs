use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;

use polo_core::{
    branch::BranchInfo,
    clock::{Clock, Hlc},
    db::{RecordParams, RecordResult, RetractParams, ScanQuery},
    error::Error,
    fact::{Attr, BranchName, EntityId, Fact, FactId, Namespace, TxId},
    gc::{GcParams, GcReport},
    merge::{ConflictEntry, ConflictResolution, DiffEntry, DiffParams, MergeParams, MergeResult},
    namespace::{MergePolicy, NamespaceInfo, NamespaceOpts},
    stats::StoreStats,
    tx::Transaction,
    Store,
};

struct State {
    facts: Vec<Fact>,
    transactions: Vec<Transaction>,
    branches: HashMap<(String, String), BranchInfo>,
    namespaces: HashMap<String, NamespaceInfo>,
    idem_cache: HashMap<String, (FactId, TxId)>,
    tags: HashMap<String, TxId>,
}

impl State {
    fn new() -> Self {
        let default_ns = NamespaceInfo {
            name: Namespace::default(),
            merge_policy: MergePolicy::default(),
            schema: None,
            created_at: Utc::now(),
        };
        let main_branch = BranchInfo {
            namespace: Namespace::default(),
            name: BranchName::main(),
            parent: None,
            fork_at: None,
            created_at: Utc::now(),
            head_tx: None,
            closed: false,
        };

        let mut namespaces = HashMap::new();
        namespaces.insert("default".into(), default_ns);

        let mut branches = HashMap::new();
        branches.insert(("default".into(), "main".into()), main_branch);

        Self {
            facts: Vec::new(),
            transactions: Vec::new(),
            branches,
            namespaces,
            idem_cache: HashMap::new(),
            tags: HashMap::new(),
        }
    }
}

pub struct MemoryStore {
    state: Arc<RwLock<State>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(State::new())),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Store for MemoryStore {
    async fn record(&self, p: RecordParams) -> Result<RecordResult, Error> {
        let mut state = self.state.write();

        if let Some(key) = &p.idempotency_key {
            if let Some((fid, tid)) = state.idem_cache.get(key) {
                return Ok(RecordResult {
                    fact_id: fid.clone(),
                    tx_id: tid.clone(),
                    was_duplicate: true,
                });
            }
        }

        let fact_id = FactId::new();
        let fact = Fact {
            id: fact_id.clone(),
            namespace: p.namespace.clone(),
            entity: p.entity,
            attr: p.attr,
            value: p.value,
            valid_from: p.valid_from,
            valid_to: p.valid_to,
            tx_id: p.tx_id.clone(),
            tx_time: p.tx_time,
            branch: p.branch.clone(),
            author: p.author.clone(),
            retracted: false,
            caused_by: p.caused_by.clone(),
        };

        state.facts.push(fact);

        // Update or create transaction record
        if let Some(tx) = state.transactions.iter_mut().find(|t| t.id == p.tx_id) {
            tx.fact_count += 1;
        } else {
            state.transactions.push(Transaction {
                id: p.tx_id.clone(),
                namespace: p.namespace.clone(),
                branch: p.branch.clone(),
                ts: p.tx_time,
                author: p.author,
                message: p.message,
                fact_count: 1,
                caused_by: p.caused_by,
            });
        }

        // Update branch head
        let key = (p.namespace.0.clone(), p.branch.0.clone());
        if let Some(branch) = state.branches.get_mut(&key) {
            branch.head_tx = Some(p.tx_id.clone());
        }

        if let Some(idem_key) = &p.idempotency_key {
            state
                .idem_cache
                .insert(idem_key.clone(), (fact_id.clone(), p.tx_id.clone()));
        }

        Ok(RecordResult {
            fact_id,
            tx_id: p.tx_id,
            was_duplicate: false,
        })
    }

    async fn retract(&self, fact_id: FactId, p: RetractParams) -> Result<TxId, Error> {
        let mut state = self.state.write();

        let fact = state
            .facts
            .iter_mut()
            .find(|f| f.id == fact_id && !f.retracted && f.branch == p.branch && f.namespace == p.namespace)
            .ok_or_else(|| Error::FactNotFound(fact_id))?;

        fact.retracted = true;
        fact.valid_to = Some(p.tx_time.to_datetime());

        state.transactions.push(Transaction {
            id: p.tx_id.clone(),
            namespace: p.namespace.clone(),
            branch: p.branch.clone(),
            ts: p.tx_time,
            author: p.author,
            message: p.message,
            fact_count: 1,
            caused_by: None,
        });

        let key = (p.namespace.0, p.branch.0);
        if let Some(branch) = state.branches.get_mut(&key) {
            branch.head_tx = Some(p.tx_id.clone());
        }

        Ok(p.tx_id)
    }

    async fn get_fact(&self, id: FactId) -> Result<Fact, Error> {
        let state = self.state.read();
        state
            .facts
            .iter()
            .find(|f| f.id == id)
            .cloned()
            .ok_or(Error::FactNotFound(id))
    }

    async fn scan(&self, q: ScanQuery) -> Result<Vec<Fact>, Error> {
        let state = self.state.read();

        let mut facts: Vec<Fact> = state
            .facts
            .iter()
            .filter(|f| {
                f.namespace == q.namespace && f.branch == q.branch
                    && q.entity.as_ref().map(|e| e == &f.entity).unwrap_or(true)
                    && q.attr.as_ref().map(|a| a == &f.attr).unwrap_or(true)
                    && (q.include_retracted || !f.retracted)
                    && q.asof_tx.map(|ts| f.tx_time <= ts).unwrap_or(true)
                    && q.asof_valid
                        .map(|at| {
                            f.valid_from <= at
                                && f.valid_to.map(|vt| at < vt).unwrap_or(true)
                        })
                        .unwrap_or(true)
            })
            .cloned()
            .collect();

        facts.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));

        let offset = q.offset.unwrap_or(0);
        let limit = q.limit.unwrap_or(usize::MAX);
        Ok(facts.into_iter().skip(offset).take(limit).collect())
    }

    async fn get_branch(&self, ns: &Namespace, name: &BranchName) -> Result<BranchInfo, Error> {
        let state = self.state.read();
        state
            .branches
            .get(&(ns.0.clone(), name.0.clone()))
            .cloned()
            .ok_or_else(|| Error::BranchNotFound(name.clone(), ns.clone()))
    }

    async fn create_branch(
        &self,
        ns: &Namespace,
        name: BranchName,
        fork_from: BranchName,
        fork_at: Hlc,
    ) -> Result<(), Error> {
        let mut state = self.state.write();

        if !state.namespaces.contains_key(ns.as_str()) {
            return Err(Error::NamespaceNotFound(ns.clone()));
        }

        let key = (ns.0.clone(), name.0.clone());
        if state.branches.contains_key(&key) {
            return Err(Error::BranchExists(name, ns.clone()));
        }

        state.branches.insert(
            key,
            BranchInfo {
                namespace: ns.clone(),
                name,
                parent: Some(fork_from),
                fork_at: Some(fork_at),
                created_at: Utc::now(),
                head_tx: None,
                closed: false,
            },
        );
        Ok(())
    }

    async fn list_branches(&self, ns: &Namespace) -> Result<Vec<BranchInfo>, Error> {
        let state = self.state.read();
        let mut branches: Vec<BranchInfo> = state
            .branches
            .iter()
            .filter(|((n, _), _)| n == ns.as_str())
            .map(|(_, b)| b.clone())
            .collect();
        branches.sort_by_key(|b| b.created_at);
        Ok(branches)
    }

    async fn delete_branch(&self, ns: &Namespace, name: &BranchName) -> Result<(), Error> {
        let mut state = self.state.write();
        let key = (ns.0.clone(), name.0.clone());

        match state.branches.get(&key) {
            None => return Err(Error::BranchNotFound(name.clone(), ns.clone())),
            Some(b) if b.parent.is_none() => {
                return Err(Error::CannotDeleteRoot(name.clone(), ns.clone()))
            }
            _ => {}
        }

        state.branches.remove(&key);
        Ok(())
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceInfo>, Error> {
        let state = self.state.read();
        let mut ns: Vec<NamespaceInfo> = state.namespaces.values().cloned().collect();
        ns.sort_by_key(|n| n.name.0.clone());
        Ok(ns)
    }

    async fn get_namespace(&self, ns: &Namespace) -> Result<NamespaceInfo, Error> {
        let state = self.state.read();
        state
            .namespaces
            .get(ns.as_str())
            .cloned()
            .ok_or_else(|| Error::NamespaceNotFound(ns.clone()))
    }

    async fn create_namespace(&self, ns: Namespace, opts: NamespaceOpts) -> Result<(), Error> {
        let mut state = self.state.write();

        if state.namespaces.contains_key(ns.as_str()) {
            return Err(Error::NamespaceExists(ns));
        }

        let ns_info = NamespaceInfo {
            name: ns.clone(),
            merge_policy: opts.merge_policy,
            schema: opts.schema,
            created_at: Utc::now(),
        };
        state.namespaces.insert(ns.0.clone(), ns_info);

        state.branches.insert(
            (ns.0.clone(), "main".into()),
            BranchInfo {
                namespace: ns,
                name: BranchName::main(),
                parent: None,
                fork_at: None,
                created_at: Utc::now(),
                head_tx: None,
                closed: false,
            },
        );
        Ok(())
    }

    async fn get_tx(&self, id: &TxId) -> Result<Transaction, Error> {
        let state = self.state.read();
        state
            .transactions
            .iter()
            .find(|t| &t.id == id)
            .cloned()
            .ok_or_else(|| Error::TxNotFound(id.clone()))
    }

    async fn list_tx(
        &self,
        ns: &Namespace,
        branch: &BranchName,
        limit: usize,
    ) -> Result<Vec<Transaction>, Error> {
        let state = self.state.read();
        let mut txs: Vec<Transaction> = state
            .transactions
            .iter()
            .filter(|t| t.namespace == *ns && t.branch == *branch)
            .cloned()
            .collect();
        txs.sort_by(|a, b| b.ts.cmp(&a.ts));
        txs.truncate(limit);
        Ok(txs)
    }

    async fn merge(&self, p: MergeParams) -> Result<MergeResult, Error> {
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

        let target_idx: HashMap<(String, String), Fact> = target_facts
            .into_iter()
            .map(|f| ((f.entity.0.clone(), f.attr.0.clone()), f))
            .collect();

        let ns_info = self.get_namespace(&p.namespace).await?;
        let policy = ns_info.merge_policy;

        let clk = Clock::new();
        let tx_id = TxId::new();
        let ts = clk.tick();
        let mut conflicts = Vec::new();
        let mut applied = 0;

        for src in &source_facts {
            let key = (src.entity.0.clone(), src.attr.0.clone());
            if let Some(tgt) = target_idx.get(&key) {
                match policy {
                    MergePolicy::ErrorOnConflict => {
                        return Err(Error::Conflict(format!(
                            "entity={} attr={}",
                            src.entity, src.attr
                        )));
                    }
                    MergePolicy::FirstWriteWins => {
                        conflicts.push(ConflictEntry {
                            entity: src.entity.to_string(),
                            attr: src.attr.to_string(),
                            source_fact: src.id.clone(),
                            target_fact: tgt.id.clone(),
                            resolution: ConflictResolution::TargetWins,
                        });
                        continue;
                    }
                    MergePolicy::LastWriteWins => {
                        if src.tx_time <= tgt.tx_time {
                            conflicts.push(ConflictEntry {
                                entity: src.entity.to_string(),
                                attr: src.attr.to_string(),
                                source_fact: src.id.clone(),
                                target_fact: tgt.id.clone(),
                                resolution: ConflictResolution::TargetWins,
                            });
                            continue;
                        }
                        conflicts.push(ConflictEntry {
                            entity: src.entity.to_string(),
                            attr: src.attr.to_string(),
                            source_fact: src.id.clone(),
                            target_fact: tgt.id.clone(),
                            resolution: ConflictResolution::SourceWins,
                        });
                    }
                }
            }

            self.record(RecordParams {
                namespace: p.namespace.clone(),
                entity: src.entity.clone(),
                attr: src.attr.clone(),
                value: src.value.clone(),
                branch: p.target.clone(),
                author: p.author.clone(),
                message: p.message.clone(),
                valid_from: src.valid_from,
                valid_to: src.valid_to,
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
        let src = self
            .scan(ScanQuery {
                namespace: p.namespace.clone(),
                branch: p.source.clone(),
                ..Default::default()
            })
            .await?;
        let tgt = self
            .scan(ScanQuery {
                namespace: p.namespace.clone(),
                branch: p.target.clone(),
                ..Default::default()
            })
            .await?;

        let mut src_idx: HashMap<(String, String), Fact> = src
            .into_iter()
            .map(|f| ((f.entity.0.clone(), f.attr.0.clone()), f))
            .collect();
        let mut tgt_idx: HashMap<(String, String), Fact> = tgt
            .into_iter()
            .map(|f| ((f.entity.0.clone(), f.attr.0.clone()), f))
            .collect();

        let mut keys: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        keys.extend(src_idx.keys().cloned());
        keys.extend(tgt_idx.keys().cloned());

        let mut entries: Vec<DiffEntry> = keys
            .into_iter()
            .filter_map(|k| {
                let s = src_idx.remove(&k);
                let t = tgt_idx.remove(&k);
                if s != t {
                    Some(DiffEntry {
                        entity: k.0,
                        attr: k.1,
                        source: s,
                        target: t,
                    })
                } else {
                    None
                }
            })
            .collect();

        entries.sort_by(|a, b| a.entity.cmp(&b.entity).then(a.attr.cmp(&b.attr)));
        Ok(entries)
    }

    async fn gc(&self, p: GcParams) -> Result<GcReport, Error> {
        let mut state = self.state.write();
        let before_ms = p.before_ms;
        let branch = p.branch.clone();

        let eligible: Vec<usize> = state
            .facts
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.retracted
                    && before_ms.map(|ms| f.tx_time.0 < ms).unwrap_or(true)
                    && branch.as_ref().map(|b| b == &f.branch).unwrap_or(true)
            })
            .map(|(i, _)| i)
            .collect();

        let facts_removed = eligible.len();

        if !p.dry_run {
            // Remove in reverse index order so earlier indices stay valid
            for i in eligible.into_iter().rev() {
                state.facts.remove(i);
            }

            // Purge transactions that no longer have any associated facts
            let live_tx_ids: std::collections::HashSet<_> =
                state.facts.iter().map(|f| f.tx_id.clone()).collect();
            let before = state.transactions.len();
            state.transactions.retain(|t| live_tx_ids.contains(&t.id));
            let tx_removed = before - state.transactions.len();

            return Ok(GcReport {
                facts_removed,
                transactions_removed: tx_removed,
                dry_run: false,
            });
        }

        Ok(GcReport {
            facts_removed,
            transactions_removed: 0,
            dry_run: true,
        })
    }

    async fn stats(&self) -> Result<StoreStats, Error> {
        let state = self.state.read();
        let facts = state.facts.iter().filter(|f| !f.retracted).count();
        let retracted = state.facts.iter().filter(|f| f.retracted).count();
        let oldest_tx = state.facts.iter().map(|f| f.tx_time.0).min().unwrap_or(0);
        let newest_tx = state.facts.iter().map(|f| f.tx_time.0).max().unwrap_or(0);

        // Rough estimate: 256 bytes per fact, 128 per transaction
        let estimated_bytes =
            (state.facts.len() * 256 + state.transactions.len() * 128) as u64;

        Ok(StoreStats {
            namespaces: state.namespaces.len(),
            branches: state.branches.len(),
            facts,
            retracted,
            transactions: state.transactions.len(),
            oldest_tx,
            newest_tx,
            estimated_bytes,
        })
    }

    async fn list_entities(
        &self,
        ns: &Namespace,
        branch: &BranchName,
    ) -> Result<Vec<EntityId>, Error> {
        let state = self.state.read();
        let mut seen = std::collections::HashSet::new();
        let mut entities = Vec::new();
        for f in state.facts.iter() {
            if f.namespace == *ns && f.branch == *branch && !f.retracted {
                if seen.insert(f.entity.0.clone()) {
                    entities.push(f.entity.clone());
                }
            }
        }
        entities.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entities)
    }

    async fn list_attrs(
        &self,
        ns: &Namespace,
        branch: &BranchName,
        entity: &EntityId,
    ) -> Result<Vec<Attr>, Error> {
        let state = self.state.read();
        let mut seen = std::collections::HashSet::new();
        let mut attrs = Vec::new();
        for f in state.facts.iter() {
            if f.namespace == *ns
                && f.branch == *branch
                && f.entity == *entity
                && !f.retracted
            {
                if seen.insert(f.attr.0.clone()) {
                    attrs.push(f.attr.clone());
                }
            }
        }
        attrs.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(attrs)
    }

    async fn put_tag(&self, label: &str, tx_id: &TxId) -> Result<(), Error> {
        let mut state = self.state.write();
        state.tags.insert(label.to_string(), tx_id.clone());
        Ok(())
    }

    async fn get_tag(&self, label: &str) -> Result<TxId, Error> {
        let state = self.state.read();
        state
            .tags
            .get(label)
            .cloned()
            .ok_or_else(|| Error::TagNotFound(label.to_string()))
    }

    async fn delete_tag(&self, label: &str) -> Result<(), Error> {
        let mut state = self.state.write();
        if state.tags.remove(label).is_none() {
            return Err(Error::TagNotFound(label.to_string()));
        }
        Ok(())
    }

    async fn list_tags(&self) -> Result<Vec<(String, TxId)>, Error> {
        let state = self.state.read();
        let mut pairs: Vec<(String, TxId)> = state
            .tags
            .iter()
            .map(|(l, t)| (l.clone(), t.clone()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(pairs)
    }

    async fn close(&self) {}
}
