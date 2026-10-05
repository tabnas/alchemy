//! Limits, metrics and cancellation.
//!
//! Limits are part of a plan, not advice: a stage that would exceed one
//! fails with [`Code::ResourceLimitExceeded`](crate::shared::Code) naming the
//! field, rather than switching to a slower or larger algorithm. Values
//! are payload bytes (UTF-8 lengths of keys and scalars, and the sum of
//! them for a retained value plus a fixed per-node allowance), which is a
//! stable and portable measure; they are not a heap measurement.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Per-run limits. Every field is named in a failure as written here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Container nesting the source may reach.
    pub max_depth: usize,
    /// Bytes in one object key.
    pub max_key_bytes: usize,
    /// Bytes in one scalar (a string's text, a number's lexeme).
    pub max_scalar_bytes: usize,
    /// Bytes retained for metadata (the column descriptors).
    pub max_metadata_bytes: usize,
    /// Columns a schema may declare.
    pub max_columns: usize,
    /// Bytes retained for one row before projection.
    pub max_record_bytes: usize,
    /// Bytes retained for any one materialized capture.
    pub max_capture_bytes: usize,
    /// Bytes the run may write; `None` for no limit.
    pub max_output_bytes: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: 256,
            max_key_bytes: 64 * 1024,
            max_scalar_bytes: 16 * 1024 * 1024,
            max_metadata_bytes: 16 * 1024 * 1024,
            max_columns: 10_000,
            max_record_bytes: 64 * 1024 * 1024,
            max_capture_bytes: 64 * 1024 * 1024,
            max_output_bytes: None,
        }
    }
}

impl Limits {
    /// No limit on anything that can be unlimited, and the largest values
    /// otherwise. For tests and trusted, measured inputs only.
    pub fn unlimited() -> Self {
        Limits {
            max_depth: usize::MAX,
            max_key_bytes: usize::MAX,
            max_scalar_bytes: usize::MAX,
            max_metadata_bytes: usize::MAX,
            max_columns: usize::MAX,
            max_record_bytes: usize::MAX,
            max_capture_bytes: usize::MAX,
            max_output_bytes: None,
        }
    }
}

/// The fixed allowance counted for every retained node (a container, a
/// member, an element) on top of its payload bytes.
pub const NODE_BYTES: usize = 16;

/// Counters and high-water marks a run reports. Shared between stages
/// through an `Arc`; every update is one relaxed atomic.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Source events seen.
    pub events: AtomicU64,
    /// Object keys seen.
    pub keys: AtomicU64,
    /// Scalars seen.
    pub scalars: AtomicU64,
    /// Rows delivered to the table protocol.
    pub rows: AtomicU64,
    /// Bytes currently held by materialized captures.
    pub captured_bytes: AtomicU64,
    /// The most bytes ever held by captures at once.
    pub captured_bytes_high: AtomicU64,
    /// The most bytes ever retained at once across every retaining stage.
    pub retained_bytes_high: AtomicU64,
    /// Bytes written to the output.
    pub output_bytes: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Metrics> {
        Arc::new(Metrics::default())
    }

    #[inline]
    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    /// Raise a high-water mark to `value` if it is higher.
    #[inline]
    pub fn raise(high: &AtomicU64, value: u64) {
        high.fetch_max(value, Ordering::Relaxed);
    }

    /// Account `bytes` as captured now, and raise the high-water marks.
    pub fn capture(&self, bytes: u64) {
        let now = self.captured_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.captured_bytes_high.fetch_max(now, Ordering::Relaxed);
        self.retained_bytes_high.fetch_max(now, Ordering::Relaxed);
    }

    /// Release `bytes` captured earlier.
    pub fn release(&self, bytes: u64) {
        self.captured_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    /// The metrics as a JSON object, one field per counter.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "events": Metrics::get(&self.events),
            "keys": Metrics::get(&self.keys),
            "scalars": Metrics::get(&self.scalars),
            "rows": Metrics::get(&self.rows),
            "captured_bytes": Metrics::get(&self.captured_bytes),
            "captured_bytes_high": Metrics::get(&self.captured_bytes_high),
            "retained_bytes_high": Metrics::get(&self.retained_bytes_high),
            "output_bytes": Metrics::get(&self.output_bytes),
        })
    }
}

/// A cancellation flag shared by the caller, the source and the stages.
///
/// The source polls it between parse steps through a parse guard and stops
/// with [`Code::Aborted`](crate::shared::Code); long loops in stages poll it too.
#[derive(Clone, Debug, Default)]
pub struct AbortFlag(Arc<AtomicBool>);

impl AbortFlag {
    pub fn new() -> Self {
        AbortFlag::default()
    }

    pub fn abort(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_tracks_high_water() {
        let m = Metrics::default();
        m.capture(10);
        m.capture(20);
        m.release(10);
        m.capture(5);
        assert_eq!(Metrics::get(&m.captured_bytes), 25);
        assert_eq!(Metrics::get(&m.captured_bytes_high), 30);
        assert_eq!(Metrics::get(&m.retained_bytes_high), 30);
        assert_eq!(m.to_json()["captured_bytes_high"], 30);
    }

    #[test]
    fn abort_is_shared() {
        let a = AbortFlag::new();
        let b = a.clone();
        assert!(!b.is_aborted());
        a.abort();
        assert!(b.is_aborted());
    }
}
