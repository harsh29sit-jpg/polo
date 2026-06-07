pub mod branch;
pub mod bulk;
pub mod clock;
pub mod db;
pub mod error;
pub mod fact;
pub mod gc;
pub mod merge;
pub mod namespace;
pub mod pql;
pub mod schema;
pub mod stats;
pub mod tx;

pub use branch::BranchInfo;
pub use bulk::{BulkResult, BulkSpec};
pub use clock::{Clock, Hlc};
pub use db::{Db, RecordOpts, RecordParams, RecordResult, RetractOpts, RetractParams, ScanQuery};
pub use error::Error;
pub use fact::{Attr, BranchName, EntityId, Fact, FactId, Namespace, TxId, Value};
pub use gc::{GcParams, GcReport, RetentionPolicy, RetentionResult};
pub use merge::{DiffEntry, DiffParams, MergeParams, MergeResult};
pub use namespace::{MergePolicy, NamespaceInfo, NamespaceOpts};
pub use schema::Schema;
pub use stats::StoreStats;
pub use tx::Transaction;

#[async_trait::async_trait]
pub trait Store: Send + Sync + 'static {
    async fn record(&self, p: RecordParams) -> Result<RecordResult, Error>;
    async fn retract(&self, fact_id: FactId, p: RetractParams) -> Result<TxId, Error>;
    async fn get_fact(&self, id: FactId) -> Result<Fact, Error>;
    async fn scan(&self, q: ScanQuery) -> Result<Vec<Fact>, Error>;

    async fn get_branch(&self, ns: &Namespace, name: &BranchName) -> Result<BranchInfo, Error>;
    async fn create_branch(
        &self,
        ns: &Namespace,
        name: BranchName,
        fork_from: BranchName,
        fork_at: Hlc,
    ) -> Result<(), Error>;
    async fn list_branches(&self, ns: &Namespace) -> Result<Vec<BranchInfo>, Error>;
    async fn delete_branch(&self, ns: &Namespace, name: &BranchName) -> Result<(), Error>;

    async fn list_namespaces(&self) -> Result<Vec<NamespaceInfo>, Error>;
    async fn get_namespace(&self, ns: &Namespace) -> Result<NamespaceInfo, Error>;
    async fn create_namespace(&self, ns: Namespace, opts: NamespaceOpts) -> Result<(), Error>;

    async fn get_tx(&self, id: &TxId) -> Result<Transaction, Error>;
    async fn list_tx(
        &self,
        ns: &Namespace,
        branch: &BranchName,
        limit: usize,
    ) -> Result<Vec<Transaction>, Error>;

    async fn merge(&self, p: MergeParams) -> Result<MergeResult, Error>;
    async fn diff(&self, p: DiffParams) -> Result<Vec<DiffEntry>, Error>;

    /// Garbage-collect retracted facts older than the given cutoff.
    async fn gc(&self, p: GcParams) -> Result<GcReport, Error>;

    /// Aggregate statistics for the whole store (all namespaces).
    async fn stats(&self) -> Result<StoreStats, Error>;

    /// List all distinct entity IDs that have at least one live fact on this branch.
    async fn list_entities(
        &self,
        ns: &Namespace,
        branch: &BranchName,
    ) -> Result<Vec<EntityId>, Error>;

    /// List all distinct attrs for an entity on a branch.
    async fn list_attrs(
        &self,
        ns: &Namespace,
        branch: &BranchName,
        entity: &EntityId,
    ) -> Result<Vec<Attr>, Error>;

    // Tag management (global across namespaces, as in aeon)
    async fn put_tag(&self, label: &str, tx_id: &TxId) -> Result<(), Error>;
    async fn get_tag(&self, label: &str) -> Result<TxId, Error>;
    async fn delete_tag(&self, label: &str) -> Result<(), Error>;
    async fn list_tags(&self) -> Result<Vec<(String, TxId)>, Error>;

    async fn close(&self);
}
