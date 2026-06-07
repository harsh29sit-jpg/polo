use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreStats {
    pub namespaces: usize,
    pub branches: usize,
    pub facts: usize,
    pub retracted: usize,
    pub transactions: usize,
    /// Oldest tx HLC seen (0 if no facts).
    pub oldest_tx: u64,
    /// Newest tx HLC seen (0 if no facts).
    pub newest_tx: u64,
    /// Approximate storage size in bytes (best-effort; 0 if unknown).
    pub estimated_bytes: u64,
}

impl std::fmt::Display for StoreStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "namespaces:   {}", self.namespaces)?;
        writeln!(f, "branches:     {}", self.branches)?;
        writeln!(f, "facts:        {}", self.facts)?;
        writeln!(f, "retracted:    {}", self.retracted)?;
        writeln!(f, "transactions: {}", self.transactions)?;
        if self.oldest_tx > 0 {
            writeln!(f, "tx range:     {:016x} ... {:016x}", self.oldest_tx, self.newest_tx)?;
        }
        if self.estimated_bytes > 0 {
            writeln!(f, "storage:      {} KiB", self.estimated_bytes / 1024)?;
        }
        Ok(())
    }
}
