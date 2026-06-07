use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

// Hybrid Logical Clock (Kulkarni & Demirbas, 2014).
//
// Layout of the u64 — 48 bits physical (ms since unix epoch) + 16 bits logical counter.
// 48 bits of ms gives ~8919 years of range. The logical counter lets us generate up to
// 65535 distinct timestamps per millisecond per node without advancing wall time.
//
// We encode this in a single u64 so HLC timestamps compare like integers: later
// always greater, and equal physical ms with higher logical is still greater.

const LOGICAL_BITS: u64 = 16;
const LOGICAL_MASK: u64 = (1 << LOGICAL_BITS) - 1;

// If we see an incoming HLC more than 1 minute ahead of local wall time we reject it.
// Legitimate NTP jitter is well below this.
const MAX_DRIFT_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Hlc(pub u64);

impl Hlc {
    pub fn from_parts(physical_ms: u64, logical: u16) -> Self {
        Self((physical_ms << LOGICAL_BITS) | u64::from(logical))
    }

    pub fn physical_ms(&self) -> u64 {
        self.0 >> LOGICAL_BITS
    }

    pub fn logical(&self) -> u16 {
        (self.0 & LOGICAL_MASK) as u16
    }

    pub fn to_datetime(&self) -> chrono::DateTime<Utc> {
        chrono::DateTime::from_timestamp_millis(self.physical_ms() as i64).unwrap_or_default()
    }

    pub fn zero() -> Self {
        Self(0)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::LowerHex for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl std::str::FromStr for Hlc {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u64::from_str_radix(s.trim_start_matches("0x"), 16).map(Hlc)
    }
}

#[derive(Clone)]
pub struct Clock {
    state: Arc<AtomicU64>,
}

impl Clock {
    pub fn new() -> Self {
        let now_ms = Utc::now().timestamp_millis() as u64;
        Self {
            state: Arc::new(AtomicU64::new(Hlc::from_parts(now_ms, 0).0)),
        }
    }

    /// Generate a new HLC for a locally-originated event.
    pub fn tick(&self) -> Hlc {
        loop {
            let wall_ms = Utc::now().timestamp_millis() as u64;
            let cur = Hlc(self.state.load(Ordering::Acquire));

            let (new_phys, new_logi) = if wall_ms > cur.physical_ms() {
                (wall_ms, 0u16)
            } else if cur.logical() < u16::MAX {
                (cur.physical_ms(), cur.logical() + 1)
            } else {
                // Logical counter saturated. Spin until the wall clock advances.
                std::hint::spin_loop();
                continue;
            };

            let next = Hlc::from_parts(new_phys, new_logi);
            if self
                .state
                .compare_exchange(cur.0, next.0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return next;
            }
        }
    }

    /// Update the clock after receiving an HLC from a remote node, then return
    /// the new local timestamp. Returns an error if the received clock is
    /// unreasonably far ahead (possible replay or misconfigured remote).
    pub fn observe(&self, received: Hlc) -> Result<Hlc, ClockError> {
        let wall_ms = Utc::now().timestamp_millis() as u64;

        if received.physical_ms() > wall_ms + MAX_DRIFT_MS {
            return Err(ClockError::ExcessiveDrift {
                received_ms: received.physical_ms(),
                local_ms: wall_ms,
            });
        }

        loop {
            let cur = Hlc(self.state.load(Ordering::Acquire));
            let max_phys = wall_ms.max(received.physical_ms()).max(cur.physical_ms());

            let new_logi = if max_phys == received.physical_ms() && max_phys == cur.physical_ms() {
                received.logical().max(cur.logical()).saturating_add(1)
            } else if max_phys == received.physical_ms() {
                received.logical().saturating_add(1)
            } else if max_phys == cur.physical_ms() {
                cur.logical().saturating_add(1)
            } else {
                0
            };

            let next = Hlc::from_parts(max_phys, new_logi);
            if self
                .state
                .compare_exchange(cur.0, next.0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(next);
            }
        }
    }

    pub fn now(&self) -> Hlc {
        Hlc(self.state.load(Ordering::Acquire))
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClockError {
    #[error(
        "received HLC is {received_ms}ms ahead of local wall clock ({local_ms}ms) — possible replay or clock skew"
    )]
    ExcessiveDrift { received_ms: u64, local_ms: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_is_monotonic() {
        let clk = Clock::new();
        let mut prev = clk.tick();
        for _ in 0..1000 {
            let next = clk.tick();
            assert!(next > prev, "HLC went backwards: {:?} after {:?}", next, prev);
            prev = next;
        }
    }

    #[test]
    fn hlc_roundtrip_display() {
        let h = Hlc::from_parts(1_700_000_000_000, 42);
        let s = h.to_string();
        let parsed: Hlc = s.parse().unwrap();
        assert_eq!(h, parsed);
    }

    #[test]
    fn observe_rejects_excessive_drift() {
        let clk = Clock::new();
        let future = Hlc::from_parts(
            Utc::now().timestamp_millis() as u64 + MAX_DRIFT_MS + 1,
            0,
        );
        assert!(clk.observe(future).is_err());
    }
}
