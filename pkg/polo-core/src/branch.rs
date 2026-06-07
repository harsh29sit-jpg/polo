use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{clock::Hlc, fact::{BranchName, Namespace, TxId}};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub namespace: Namespace,
    pub name: BranchName,
    pub parent: Option<BranchName>,
    /// The HLC at which this branch forked from its parent.
    pub fork_at: Option<Hlc>,
    pub created_at: DateTime<Utc>,
    pub head_tx: Option<TxId>,
    pub closed: bool,
}

impl BranchInfo {
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }
}
