#![allow(dead_code)]
//! Signal watchlist — tracks high-confidence SKIPs across cycles.
//!
//! When `make_decision` returns a SKIP with confidence above the watch threshold,
//! or when a BUY/SELL is gated (funding stale, circuit breaker, etc.), the signal
//! is placed on a watchlist rather than discarded.  On every subsequent cycle where
//! that symbol appears in the candidate list, the watchlist entry is re-evaluated
//! against the current decision and price.
//!
//! # Three outcomes per re-evaluation
//!
//! | Condition | Status | Log |
//! |-----------|--------|-----|
//! | Same direction, confidence ≥ original, price within ±1.5% | `StillViable` | "🔄 signal holding" |
//! | Same direction, confidence ≥ original + 8% | `Strengthened` | "⬆️  signal improved" |
//! | Price drifted >1.5% against intended direction OR confidence flipped OR >MAX_WATCH_CYCLES elapsed | `Expired` | "⌛ missed / expired" |
//!
//! An entry is removed from the watchlist once it is `Expired`, or when an actual
//! trade fires for that symbol (making the question moot).
//!
//! # Noise control
//!
//! Only SKIPs above `WATCH_CONFIDENCE_FLOOR` (40 %) enter the watchlist.
//! Pure-neutral SKIPs (confidence ≈ 50/50) are not tracked — only directionally
//! biased signals that nearly crossed the entry threshold.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

// ─────────────────────────── Constants ───────────────────────────────────────

/// Minimum confidence for a SKIP to be watchlisted.
/// Below this the signal has no meaningful directional bias.
const WATCH_CONFIDENCE_FLOOR: f64 = 0.40;

/// Maximum number of 30-second cycles a signal is watched before expiry.
/// 40 cycles × 30 s = 20 minutes — beyond this the setup has meaningfully changed.
const MAX_WATCH_CYCLES: u32 = 40;

/// If the current price has moved this far (%) against the intended direction
/// the entry window is considered missed and the signal is expired.
const PRICE_DRIFT_LIMIT_PCT: f64 = 1.5;

/// Confidence improvement threshold to flag a signal as "strengthened".
const STRENGTHEN_DELTA: f64 = 0.08;

// ─────────────────────────── Types ───────────────────────────────────────────

/// Why the signal was originally skipped.
#[derive(Debug, Clone, PartialEq)]
pub enum SkipReason {
    /// Decision engine scored below MIN_CONFIDENCE.
    LowConfidence,
    /// A hard gate fired (funding stale, CB active, etc.).
    Gated(String),
}

/// Current lifecycle state of a watched signal.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchStatus {
    /// Signal is still pointing the same direction within the price window.
    Watching,
    /// Signal direction flipped, price drifted too far, or age limit reached.
    Expired,
}

/// One watched signal entry.
#[derive(Debug, Clone)]
pub struct WatchedSignal {
    pub symbol:              String,
    /// "BUY" or "SELL" — the direction the signal was pointing at watch time.
    pub action:              String,
    pub skip_reason:         SkipReason,
    pub original_confidence: f64,
    /// Price at the time the signal was first watchlisted.
    pub original_price:      f64,
    pub queued_at:           Instant,
    pub cycles_watched:      u32,
    /// Most recent re-evaluation confidence (updated each cycle).
    pub last_confidence:     f64,
    pub status:              WatchStatus,
}

impl WatchedSignal {
    /// Returns whether the entry window is still open.
    /// "Open" means price hasn't drifted beyond `PRICE_DRIFT_LIMIT_PCT` against
    /// the intended direction.
    pub fn price_still_valid(&self, current_price: f64) -> bool {
        if self.original_price <= 0.0 { return false; }
        let drift_pct = (current_price - self.original_price) / self.original_price * 100.0;
        match self.action.as_str() {
            "BUY"  => drift_pct < PRICE_DRIFT_LIMIT_PCT,   // price ran up too far → missed
            "SELL" => drift_pct > -PRICE_DRIFT_LIMIT_PCT,  // price fell too far → missed
            _      => false,
        }
    }

    /// Human-readable age string.
    pub fn age_str(&self) -> String {
        let secs = self.queued_at.elapsed().as_secs();
        if secs < 60 { format!("{}s", secs) }
        else { format!("{}m{}s", secs / 60, secs % 60) }
    }
}

// ─────────────────────────── Cache ───────────────────────────────────────────

pub struct SignalWatchlist {
    /// Keyed by symbol.  One entry per symbol — newer signals replace older ones.
    inner: RwLock<HashMap<String, WatchedSignal>>,
}

pub type SharedWatchlist = Arc<SignalWatchlist>;

impl SignalWatchlist {
    pub fn new() -> SharedWatchlist {
        Arc::new(SignalWatchlist {
            inner: RwLock::new(HashMap::new()),
        })
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Called after `analyse_symbol` returns a SKIP.
    /// If the SKIP has a directional bias (confidence ≥ `WATCH_CONFIDENCE_FLOOR`)
    /// it is added to the watchlist.  Existing entries for the same symbol are
    /// replaced only if the new signal is higher confidence.
    pub async fn maybe_watch(
        &self,
        symbol:     &str,
        action:     &str,       // "BUY" | "SELL"
        confidence: f64,
        price:      f64,
        reason:     SkipReason,
    ) {
        // Only directionally biased signals are worth watching.
        if confidence < WATCH_CONFIDENCE_FLOOR { return; }
        if action == "SKIP" { return; }

        let mut w = self.inner.write().await;

        // If there's already a higher-confidence entry for the same symbol,
        // don't downgrade it.
        if let Some(existing) = w.get(symbol) {
            if existing.status == WatchStatus::Watching
                && existing.original_confidence >= confidence
                && existing.action == action
            {
                return;
            }
        }

        log::info!(
            "👁  Watchlisting {} {} conf={:.0}% @ ${:.4} ({})",
            action, symbol, confidence * 100.0, price,
            match &reason {
                SkipReason::LowConfidence => "below threshold".to_string(),
                SkipReason::Gated(r) => r.clone(),
            }
        );

        w.insert(symbol.to_string(), WatchedSignal {
            symbol:              symbol.to_string(),
            action:              action.to_string(),
            skip_reason:         reason,
            original_confidence: confidence,
            original_price:      price,
            queued_at:           Instant::now(),
            cycles_watched:      0,
            last_confidence:     confidence,
            status:              WatchStatus::Watching,
        });
    }

    /// Called on each subsequent cycle for a symbol that is on the watchlist.
    /// Updates the entry and logs the outcome.  Returns a `WatchOutcome` summary.
    pub async fn re_evaluate(
        &self,
        symbol:             &str,
        current_action:     &str,   // current cycle's decision action
        current_confidence: f64,
        current_price:      f64,
    ) -> Option<WatchOutcome> {
        let mut w = self.inner.write().await;
        let entry = w.get_mut(symbol)?;

        if entry.status == WatchStatus::Expired { return None; }

        entry.cycles_watched  += 1;
        entry.last_confidence  = current_confidence;

        // ── Check expiry conditions ───────────────────────────────────────

        let direction_flipped = current_action != "SKIP"
            && current_action != entry.action;

        let age_exceeded = entry.cycles_watched >= MAX_WATCH_CYCLES;

        let price_missed = !entry.price_still_valid(current_price);

        if direction_flipped || age_exceeded || price_missed {
            let reason = if direction_flipped {
                format!("direction flipped to {}", current_action)
            } else if age_exceeded {
                format!("timed out after {} cycles", entry.cycles_watched)
            } else {
                let drift = (current_price - entry.original_price)
                    / entry.original_price * 100.0;
                format!("price drifted {:.2}% — entry window missed", drift)
            };

            log::info!(
                "⌛ {} {} watchlist expired: {} (was conf={:.0}%, age={})",
                entry.action, symbol, reason,
                entry.original_confidence * 100.0,
                entry.age_str()
            );

            entry.status = WatchStatus::Expired;
            return Some(WatchOutcome::Expired { reason });
        }

        // ── Still alive: classify the current state ───────────────────────

        let same_direction  = current_action == entry.action
            || current_action == "SKIP"; // SKIP = not contradicting

        let strengthened = same_direction
            && current_confidence >= entry.original_confidence + STRENGTHEN_DELTA;

        let still_viable = same_direction && current_confidence >= WATCH_CONFIDENCE_FLOOR;

        if strengthened {
            log::info!(
                "⬆️  {} {} signal strengthened: conf {:.0}%→{:.0}% age={} price={:.4}",
                entry.action, symbol,
                entry.original_confidence * 100.0, current_confidence * 100.0,
                entry.age_str(), current_price
            );
            Some(WatchOutcome::Strengthened {
                original_confidence: entry.original_confidence,
                current_confidence,
                age_str: entry.age_str(),
            })
        } else if still_viable {
            log::debug!(
                "🔄 {} {} still viable: conf={:.0}% age={} price={:.4}",
                entry.action, symbol, current_confidence * 100.0,
                entry.age_str(), current_price
            );
            Some(WatchOutcome::StillViable {
                current_confidence,
                age_str: entry.age_str(),
            })
        } else {
            // Confidence dropped below floor (signal fading without reversing).
            log::info!(
                "📉 {} {} faded: conf {:.0}%→{:.0}% — expiring",
                entry.action, symbol,
                entry.original_confidence * 100.0, current_confidence * 100.0,
            );
            entry.status = WatchStatus::Expired;
            Some(WatchOutcome::Expired {
                reason: format!(
                    "confidence faded from {:.0}% to {:.0}%",
                    entry.original_confidence * 100.0, current_confidence * 100.0
                ),
            })
        }
    }

    /// Remove the watchlist entry for a symbol when a real trade fires.
    pub async fn remove(&self, symbol: &str) {
        self.inner.write().await.remove(symbol);
    }

    /// Snapshot of all currently-watching entries (excludes expired).
    pub async fn watching(&self) -> Vec<WatchedSignal> {
        self.inner.read().await
            .values()
            .filter(|e| e.status == WatchStatus::Watching)
            .cloned()
            .collect()
    }

    /// Count of active (non-expired) entries.
    pub async fn active_count(&self) -> usize {
        self.inner.read().await
            .values()
            .filter(|e| e.status == WatchStatus::Watching)
            .count()
    }

    /// Evict all expired entries to keep the map tidy.
    /// Called once per cycle at the end of the candidate loop.
    pub async fn gc(&self) {
        let mut w = self.inner.write().await;
        w.retain(|_, v| v.status == WatchStatus::Watching);
    }
}

// ─────────────────────────── Outcome type ────────────────────────────────────

/// Result returned by `re_evaluate()`.
#[derive(Debug)]
pub enum WatchOutcome {
    StillViable {
        current_confidence: f64,
        age_str:            String,
    },
    Strengthened {
        original_confidence: f64,
        current_confidence:  f64,
        age_str:             String,
    },
    Expired {
        reason: String,
    },
}

impl WatchOutcome {
    pub fn is_strengthened(&self) -> bool {
        matches!(self, WatchOutcome::Strengthened { .. })
    }
}

// ─────────────────────────── Unit tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    fn rt() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn watch_threshold_filters_low_confidence() {
        let rt = rt();
        rt.block_on(async {
            let wl = SignalWatchlist::new();
            // 35% is below floor — should not be watchlisted
            wl.maybe_watch("ETH", "BUY", 0.35, 3000.0, SkipReason::LowConfidence).await;
            assert_eq!(wl.active_count().await, 0);
        });
    }

    #[test]
    fn watch_above_threshold_is_added() {
        let rt = rt();
        rt.block_on(async {
            let wl = SignalWatchlist::new();
            wl.maybe_watch("ETH", "BUY", 0.55, 3000.0, SkipReason::LowConfidence).await;
            assert_eq!(wl.active_count().await, 1);
        });
    }

    #[test]
    fn re_evaluate_still_viable() {
        let rt = rt();
        rt.block_on(async {
            let wl = SignalWatchlist::new();
            wl.maybe_watch("SOL", "BUY", 0.55, 100.0, SkipReason::LowConfidence).await;
            let outcome = wl.re_evaluate("SOL", "SKIP", 0.55, 100.5).await;
            assert!(matches!(outcome, Some(WatchOutcome::StillViable { .. })));
        });
    }

    #[test]
    fn re_evaluate_strengthened_when_confidence_jumps() {
        let rt = rt();
        rt.block_on(async {
            let wl = SignalWatchlist::new();
            wl.maybe_watch("BTC", "BUY", 0.55, 50000.0, SkipReason::LowConfidence).await;
            // Confidence jumped +10% → strengthened
            let outcome = wl.re_evaluate("BTC", "BUY", 0.65, 50100.0).await;
            assert!(matches!(outcome, Some(WatchOutcome::Strengthened { .. })));
        });
    }

    #[test]
    fn re_evaluate_expires_on_price_drift() {
        let rt = rt();
        rt.block_on(async {
            let wl = SignalWatchlist::new();
            wl.maybe_watch("BTC", "BUY", 0.55, 50000.0, SkipReason::LowConfidence).await;
            // Price ran up 2% — entry window missed for a BUY
            let outcome = wl.re_evaluate("BTC", "BUY", 0.60, 51000.0).await;
            assert!(matches!(outcome, Some(WatchOutcome::Expired { .. })));
            assert_eq!(wl.active_count().await, 0);
        });
    }

    #[test]
    fn re_evaluate_expires_on_direction_flip() {
        let rt = rt();
        rt.block_on(async {
            let wl = SignalWatchlist::new();
            wl.maybe_watch("BTC", "BUY", 0.60, 50000.0, SkipReason::LowConfidence).await;
            // Signal now says SELL → expired
            let outcome = wl.re_evaluate("BTC", "SELL", 0.62, 50100.0).await;
            assert!(matches!(outcome, Some(WatchOutcome::Expired { .. })));
        });
    }

    #[test]
    fn price_drift_logic_correct_direction() {
        let sig = WatchedSignal {
            symbol:              "X".into(),
            action:              "BUY".into(),
            skip_reason:         SkipReason::LowConfidence,
            original_confidence: 0.55,
            original_price:      100.0,
            queued_at:           Instant::now(),
            cycles_watched:      0,
            last_confidence:     0.55,
            status:              WatchStatus::Watching,
        };
        // +1.0% up → still within 1.5% limit for a BUY
        assert!(sig.price_still_valid(101.0));
        // +2.0% up → ran away, entry missed
        assert!(!sig.price_still_valid(102.1));
        // -2.0% down → going against us but still in window (price hasn't run yet)
        assert!(sig.price_still_valid(98.0));
    }
}
