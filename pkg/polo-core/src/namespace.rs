use serde::{Deserialize, Serialize};

use crate::fact::Namespace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergePolicy {
    #[default]
    LastWriteWins,
    FirstWriteWins,
    ErrorOnConflict,
}

impl std::fmt::Display for MergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergePolicy::LastWriteWins => f.write_str("last_write_wins"),
            MergePolicy::FirstWriteWins => f.write_str("first_write_wins"),
            MergePolicy::ErrorOnConflict => f.write_str("error_on_conflict"),
        }
    }
}

impl std::str::FromStr for MergePolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "last_write_wins" | "last-write-wins" => Ok(MergePolicy::LastWriteWins),
            "first_write_wins" | "first-write-wins" => Ok(MergePolicy::FirstWriteWins),
            "error_on_conflict" | "error-on-conflict" => Ok(MergePolicy::ErrorOnConflict),
            _ => Err(format!("unknown merge policy '{s}'")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceInfo {
    pub name: Namespace,
    pub merge_policy: MergePolicy,
    pub schema: Option<crate::schema::Schema>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamespaceOpts {
    pub merge_policy: MergePolicy,
    pub schema: Option<crate::schema::Schema>,
}
