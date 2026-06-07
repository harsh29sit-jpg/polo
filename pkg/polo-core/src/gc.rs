use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::fact::BranchName;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GcParams {
    /// Remove retracted facts whose tx_time is before this cutoff (in ms since epoch).
    /// Zero means no age-based cutoff.
    pub before_ms: Option<u64>,
    /// Only process this branch. None = all branches.
    pub branch: Option<BranchName>,
    /// When true, report what would be removed but don't actually delete.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GcReport {
    pub facts_removed: usize,
    pub transactions_removed: usize,
    pub dry_run: bool,
}

// ---- Retention (higher-level, applied at the Db layer) ----

#[derive(Debug, Clone, Default)]
pub struct RetentionPolicy {
    /// Maximum age of a fact (in seconds). Facts older than this are eligible for removal.
    pub max_age_secs: Option<u64>,
    /// Keep at most this many versions of any (entity, attr) pair per branch.
    /// Older versions beyond this count become eligible.
    pub max_versions: Option<usize>,
    /// When true, only consider already-retracted facts.
    pub only_retracted: bool,
    /// Limit to these branches. Empty = all branches.
    pub branches: Vec<BranchName>,
    /// Report without applying.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetentionResult {
    pub inspected: usize,
    pub eligible: usize,
    pub retracted: usize,
    pub branches_processed: usize,
}
