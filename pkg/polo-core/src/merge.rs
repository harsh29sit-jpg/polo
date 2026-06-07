use serde::{Deserialize, Serialize};

use crate::{clock::Hlc, fact::{BranchName, Fact, FactId, Namespace, TxId}};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeParams {
    pub namespace: Namespace,
    pub source: BranchName,
    pub target: BranchName,
    pub author: Option<String>,
    pub message: Option<String>,
    pub caused_by: Option<TxId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    pub tx_id: TxId,
    pub ts: Hlc,
    pub facts_applied: usize,
    pub conflicts: Vec<ConflictEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictEntry {
    pub entity: String,
    pub attr: String,
    pub source_fact: FactId,
    pub target_fact: FactId,
    pub resolution: ConflictResolution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    SourceWins,
    TargetWins,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffParams {
    pub namespace: Namespace,
    pub source: BranchName,
    pub target: BranchName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    pub entity: String,
    pub attr: String,
    pub source: Option<Fact>,
    pub target: Option<Fact>,
}
