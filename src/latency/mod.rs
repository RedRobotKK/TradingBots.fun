//! Latency benchmarking module.
//!
//! Tracks 5 timing primitives per trade and computes rolling percentiles
//! (p50, p95, p99) per session. Exposed via `GET /api/v1/session/{id}/latency/stats`.
//!
//! ## Design
//! - Uses `std::time::Instant` (monotonic) — never `SystemTime`.
//! - Measurements are stored in an in-memory ring buffer (last 1000 per session).
//! - Background aggregation is event-driven (called on each `record_fill`).
//! - Zero-copy design: DashMap is not used to avoid adding a dependency;
//!   instead we use `tokio::sync::RwLock<HashMap<...>>` which is already in scope.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

// ─────────────────────────── Primitives ──────────────────────────────────────

/// The 5 timing primitives recorded per trade.
#[derive(Debug, Clone)]
pub struct TradeLatencyRecord {
    pub trade_id:              String,
    pub coin:                  String,
    /// When the AI/strategy emitted the signal.
    pub signal_received_at:    Instant,
    /// After the order was signed (EIP-712 / HL phantom-agent).
    pub order_signed_at:       Option<Instant>,
    /// When the HTTP request left our connector.
    pub order_sent_at:         Option<Instant>,
    /// First response / ack received from Hyperliquid.
    pub response_received_at:  Option<Instant>,
    /// When the fill was confirmed (via WS event or /userState poll).
    pub fill_confirmed_at:     Option<Instant>,
}

impl TradeLatencyRecord {
    /// Create a new record stamped at "now".
    pub fn new(trade_id: impl Into<String>, coin: impl Into<String>) -> Self {
        TradeLatencyRecord {
            trade_id:             trade_id.into(),
            coin:                 coin.into(),
            signal_received_at:   Instant::now(),
            order_signed_at:      None,
            order_sent_at:        None,
            response_received_at: None,
            fill_confirmed_at:    None,
        }
    }

    /// Milliseconds from signal → order response (network RTT).
    pub fn order_latency_ms(&self) -> Option<f64> {
        let sent = self.order_sent_at?;
        let recv = self.response_received_at?;
        Some(recv.duration_since(sent).as_secs_f64() * 1000.0)
    }

    /// Milliseconds from order sent → fill confirmed.
    pub fn fill_latency_ms(&self) -> Option<f64> {
        let sent = self.order_sent_at?;
        let fill = self.fill_confirmed_at?;
        Some(fill.duration_since(sent).as_secs_f64() * 1000.0)
    }

    /// Total end-to-end latency: signal → fill confirmed.
    pub fn total_latency_ms(&self) -> Option<f64> {
        let start = self.signal_received_at;
        let fill  = self.fill_confirmed_at?;
        Some(fill.duration_since(start).as_secs_f64() * 1000.0)
    }
}

// ─────────────────────────── Aggregates ──────────────────────────────────────

/// Session-level latency statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionLatencyStats {
    pub session_id:   String,
    pub sample_count: usize,
    // Total latency percentiles (ms)
    pub p50_ms:  f64,
    pub p95_ms:  f64,
    pub p99_ms:  f64,
    pub min_ms:  f64,
    pub max_ms:  f64,
    pub mean_ms: f64,
    pub stddev_ms: f64,
    // Order-only latency (network RTT)
    pub order_p50_ms: f64,
    pub order_p95_ms: f64,
    // Throughput
    pub trades_per_minute: f64,
    /// Fraction of trades filled within 500 ms.
    pub success_rate_pct:  f64,
    /// Goal targets
    pub target_median_ms: f64,
    pub target_p95_ms:    f64,
    pub target_p99_ms:    f64,
}

impl SessionLatencyStats {
    const TARGET_MEDIAN: f64 = 250.0;
    const TARGET_P95:    f64 = 450.0;
    const TARGET_P99:    f64 = 800.0;

    /// Compute stats from a slice of total_latency_ms values.
    pub fn compute(
        session_id: &str,
        total_ms:   &mut [f64],
        order_ms:   &mut [f64],
        window_secs: f64,
    ) -> Self {
        if total_ms.is_empty() {
            return SessionLatencyStats {
                session_id:       session_id.to_string(),
                target_median_ms: Self::TARGET_MEDIAN,
                target_p95_ms:    Self::TARGET_P95,
                target_p99_ms:    Self::TARGET_P99,
                ..Default::default()
            };
        }

        total_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        order_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let n = total_ms.len();
        let percentile = |v: &[f64], pct: f64| -> f64 {
            let idx = ((pct / 100.0) * (n as f64 - 1.0)).round() as usize;
            v[idx.min(v.len().saturating_sub(1))]
        };

        let sum: f64 = total_ms.iter().sum();
        let mean = sum / n as f64;
        let variance = total_ms.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        let stddev = variance.sqrt();

        let success = total_ms.iter().filter(|&&v| v <= 500.0).count();
        let tpm = if window_secs > 0.0 { (n as f64 / window_secs) * 60.0 } else { 0.0 };

        SessionLatencyStats {
            session_id:         session_id.to_string(),
            sample_count:       n,
            p50_ms:             percentile(total_ms, 50.0),
            p95_ms:             percentile(total_ms, 95.0),
            p99_ms:             percentile(total_ms, 99.0),
            min_ms:             total_ms[0],
            max_ms:             total_ms[n - 1],
            mean_ms:            mean,
            stddev_ms:          stddev,
            order_p50_ms:       if order_ms.is_empty() { 0.0 } else { percentile(order_ms, 50.0) },
            order_p95_ms:       if order_ms.is_empty() { 0.0 } else { percentile(order_ms, 95.0) },
            trades_per_minute:  tpm,
            success_rate_pct:   (success as f64 / n as f64) * 100.0,
            target_median_ms:   Self::TARGET_MEDIAN,
            target_p95_ms:      Self::TARGET_P95,
            target_p99_ms:      Self::TARGET_P99,
        }
    }
}

// ─────────────────────────── Tracker ─────────────────────────────────────────

/// Per-session latency tracker.
///
/// Holds up to `cap` records in a ring buffer. Thread-safe for concurrent inserts
/// via interior mutability pattern — caller should wrap in `tokio::sync::Mutex`.
pub struct LatencyTracker {
    pub session_id: String,
    /// Ring buffer: (total_ms, order_ms), for completed trades only.
    measurements:   Vec<(f64, Option<f64>)>,
    /// Timestamp when this tracker was created (for TPM calculation).
    started_at:     Instant,
    cap:            usize,
}

impl LatencyTracker {
    pub fn new(session_id: impl Into<String>) -> Self {
        LatencyTracker {
            session_id:   session_id.into(),
            measurements: Vec::new(),
            started_at:   Instant::now(),
            cap:          1000,
        }
    }

    /// Record a completed trade timing.
    pub fn record(&mut self, record: &TradeLatencyRecord) {
        if let Some(total_ms) = record.total_latency_ms() {
            let order_ms = record.order_latency_ms();
            self.measurements.push((total_ms, order_ms));
            if self.measurements.len() > self.cap {
                self.measurements.remove(0);
            }
        }
    }

    /// Compute stats over all stored measurements.
    pub fn stats(&self) -> SessionLatencyStats {
        let mut total: Vec<f64> = self.measurements.iter().map(|(t, _)| *t).collect();
        let mut order: Vec<f64> = self.measurements.iter()
            .filter_map(|(_, o)| *o)
            .collect();
        let elapsed = self.started_at.elapsed().as_secs_f64();
        SessionLatencyStats::compute(&self.session_id, &mut total, &mut order, elapsed)
    }

    /// Number of measurements in the buffer.
    pub fn len(&self) -> usize {
        self.measurements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.measurements.is_empty()
    }
}

// ─────────────────────────── Registry ────────────────────────────────────────

/// Global registry of per-session latency trackers.
/// Wrap in `Arc<tokio::sync::Mutex<LatencyRegistry>>` at the call site.
pub type LatencyRegistry = HashMap<String, LatencyTracker>;
