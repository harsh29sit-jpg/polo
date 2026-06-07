use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fact::{EntityId, Attr, FactId, TxId, Value};

/// A single fact spec inside a bulk operation. All specs in one BulkRecord call
/// share the same transaction ID and timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkSpec {
    pub entity: EntityId,
    pub attr: Attr,
    pub value: Value,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkResult {
    pub tx_id: TxId,
    pub applied: usize,
    pub fact_ids: Vec<FactId>,
}
