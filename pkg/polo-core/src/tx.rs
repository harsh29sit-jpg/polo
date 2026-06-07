use serde::{Deserialize, Serialize};

use crate::{clock::Hlc, fact::{BranchName, Namespace, TxId}};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: TxId,
    pub namespace: Namespace,
    pub branch: BranchName,
    pub ts: Hlc,
    pub author: Option<String>,
    pub message: Option<String>,
    pub fact_count: usize,
    /// The transaction that caused this one, if any.
    pub caused_by: Option<TxId>,
}
