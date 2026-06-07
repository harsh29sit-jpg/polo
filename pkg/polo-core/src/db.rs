use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    branch::BranchInfo,
    clock::{Clock, Hlc},
    error::Error,
    fact::{Attr, BranchName, EntityId, Fact, FactId, Namespace, TxId, Value},
    merge::{DiffEntry, DiffParams, MergeParams, MergeResult},
    namespace::{NamespaceInfo, NamespaceOpts},
    tx::Transaction,
    Store,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordParams {
    pub namespace: Namespace,
    pub entity: EntityId,
    pub attr: Attr,
    pub value: Value,
    pub branch: BranchName,
    pub author: Option<String>,
    pub message: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub tx_time: Hlc,
    pub tx_id: TxId,
    pub caused_by: Option<TxId>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordResult {
    pub fact_id: FactId,
    pub tx_id: TxId,
    pub was_duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetractParams {
    pub namespace: Namespace,
    pub branch: BranchName,
    pub author: Option<String>,
    pub message: Option<String>,
    pub tx_time: Hlc,
    pub tx_id: TxId,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanQuery {
    pub namespace: Namespace,
    pub branch: BranchName,
    pub entity: Option<EntityId>,
    pub attr: Option<Attr>,
    /// Return only the fact valid at this transaction time (as-of semantics).
    pub asof_tx: Option<Hlc>,
    /// Return only the fact valid at this point in valid time (effective semantics).
    pub asof_valid: Option<DateTime<Utc>>,
    pub include_retracted: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub struct Db {
    store: Arc<dyn Store>,
    clock: Clock,
}

impl Db {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            clock: Clock::new(),
        }
    }

    pub fn with_clock(store: Arc<dyn Store>, clock: Clock) -> Self {
        Self { store, clock }
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    pub async fn record(
        &self,
        ns: Namespace,
        entity: EntityId,
        attr: Attr,
        value: Value,
        branch: BranchName,
        opts: RecordOpts,
    ) -> Result<RecordResult, Error> {
        let valid_from = opts.valid_from.unwrap_or_else(Utc::now);
        if let Some(valid_to) = opts.valid_to {
            if valid_to <= valid_from {
                return Err(Error::InvalidTimeRange);
            }
        }

        // Validate against namespace schema if one is set.
        if let Ok(ns_info) = self.store.get_namespace(&ns).await {
            if let Some(schema) = &ns_info.schema {
                schema.validate(&attr, &value)?;
            }
        }

        let tx_time = self.clock.tick();
        let tx_id = TxId::new();

        self.store
            .record(RecordParams {
                namespace: ns,
                entity,
                attr,
                value,
                branch,
                author: opts.author,
                message: opts.message,
                valid_from,
                valid_to: opts.valid_to,
                tx_time,
                tx_id,
                caused_by: opts.caused_by,
                idempotency_key: opts.idempotency_key,
            })
            .await
    }

    pub async fn retract(
        &self,
        ns: Namespace,
        fact_id: FactId,
        branch: BranchName,
        opts: RetractOpts,
    ) -> Result<TxId, Error> {
        let tx_time = self.clock.tick();
        let tx_id = TxId::new();
        self.store
            .retract(
                fact_id,
                RetractParams {
                    namespace: ns,
                    branch,
                    author: opts.author,
                    message: opts.message,
                    tx_time,
                    tx_id,
                },
            )
            .await
    }

    pub async fn get_fact(&self, id: FactId) -> Result<Fact, Error> {
        self.store.get_fact(id).await
    }

    pub async fn scan(&self, q: ScanQuery) -> Result<Vec<Fact>, Error> {
        self.store.scan(q).await
    }

    pub async fn history(
        &self,
        ns: Namespace,
        branch: BranchName,
        entity: EntityId,
        attr: Attr,
    ) -> Result<Vec<Fact>, Error> {
        self.store
            .scan(ScanQuery {
                namespace: ns,
                branch,
                entity: Some(entity),
                attr: Some(attr),
                include_retracted: true,
                ..Default::default()
            })
            .await
    }

    pub async fn effective(
        &self,
        ns: Namespace,
        branch: BranchName,
        entity: EntityId,
        attr: Attr,
        at: DateTime<Utc>,
    ) -> Result<Option<Fact>, Error> {
        let facts = self
            .store
            .scan(ScanQuery {
                namespace: ns,
                branch,
                entity: Some(entity),
                attr: Some(attr),
                asof_valid: Some(at),
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        Ok(facts.into_iter().next())
    }

    pub async fn asof(
        &self,
        ns: Namespace,
        branch: BranchName,
        entity: EntityId,
        attr: Attr,
        at: Hlc,
    ) -> Result<Option<Fact>, Error> {
        let facts = self
            .store
            .scan(ScanQuery {
                namespace: ns,
                branch,
                entity: Some(entity),
                attr: Some(attr),
                asof_tx: Some(at),
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        Ok(facts.into_iter().next())
    }

    pub async fn snapshot(
        &self,
        ns: Namespace,
        branch: BranchName,
        entity: EntityId,
    ) -> Result<Vec<Fact>, Error> {
        self.store
            .scan(ScanQuery {
                namespace: ns,
                branch,
                entity: Some(entity),
                ..Default::default()
            })
            .await
    }

    pub async fn create_namespace(&self, ns: Namespace, opts: NamespaceOpts) -> Result<(), Error> {
        self.store.create_namespace(ns, opts).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<NamespaceInfo>, Error> {
        self.store.list_namespaces().await
    }

    pub async fn get_namespace(&self, ns: &Namespace) -> Result<NamespaceInfo, Error> {
        self.store.get_namespace(ns).await
    }

    pub async fn create_branch(
        &self,
        ns: &Namespace,
        name: BranchName,
        fork_from: BranchName,
    ) -> Result<(), Error> {
        let fork_at = self.clock.tick();
        self.store.create_branch(ns, name, fork_from, fork_at).await
    }

    pub async fn list_branches(&self, ns: &Namespace) -> Result<Vec<BranchInfo>, Error> {
        self.store.list_branches(ns).await
    }

    pub async fn get_branch(&self, ns: &Namespace, name: &BranchName) -> Result<BranchInfo, Error> {
        self.store.get_branch(ns, name).await
    }

    pub async fn delete_branch(&self, ns: &Namespace, name: &BranchName) -> Result<(), Error> {
        self.store.delete_branch(ns, name).await
    }

    pub async fn merge(&self, p: MergeParams) -> Result<MergeResult, Error> {
        self.store.merge(p).await
    }

    pub async fn diff(&self, p: DiffParams) -> Result<Vec<DiffEntry>, Error> {
        self.store.diff(p).await
    }

    pub async fn get_tx(&self, id: &TxId) -> Result<Transaction, Error> {
        self.store.get_tx(id).await
    }

    pub async fn list_tx(
        &self,
        ns: &Namespace,
        branch: &BranchName,
        limit: usize,
    ) -> Result<Vec<Transaction>, Error> {
        self.store.list_tx(ns, branch, limit).await
    }

    pub async fn close(&self) {
        self.store.close().await;
    }
}

#[derive(Debug, Default, Clone)]
pub struct RecordOpts {
    pub author: Option<String>,
    pub message: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub caused_by: Option<TxId>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct RetractOpts {
    pub author: Option<String>,
    pub message: Option<String>,
}
