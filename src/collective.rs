//! Collective intelligence — cross-user learning layer.
//!
//! Every closed trade is recorded in `trade_outcomes` with the alignment of
//! each signal relative to the trade direction (+1 agreed, -1 opposed, 0 absent).
//! Every open position is tracked in `hot_positions`.
//!
//! The trading loop uses two outputs from this module on every cycle:
//!
//! ## 1 — Crowd signal (`get_crowd_signal`)
//!
//! Before entering a new position the bot queries `hot_positions` for the
//! target symbol.  If multiple users are already holding the same side AND
//! are profitable, confidence is nudged up slightly (confirming signal).
//! If multiple users are losing on the same side, confidence is nudged down
//! (crowd got trapped — caution).
//!
//! ```
//! let crowd = collective::get_crowd_signal(&db, "SOL").await;
//! let mult  = crowd.as_ref().map(|c| c.confidence_multiplier("LONG")).unwrap_or(1.0);
//! dec.confidence *= mult;
//! ```
//!
//! ## 2 — Nightly weight recalculation (`recalculate_collective_weights`)
//!
//! Called at midnight by the daily analyst task.  Aggregates signal precision
//! from the last 90 days of outcomes (all tenants combined) and blends the
//! result into the shared `SignalWeights`.
//!
//! More users → more outcomes → faster convergence on what actually works.
//! The blend ratio is 60% collective / 40% current to prevent sudden swings.

use crate::db::Database;
use crate::learner::SignalWeights;
use crate::web_dashboard::PaperPosition;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
//  Crowd signal
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregate view of all users currently holding a position in one symbol.
#[derive(Debug, Clone)]
pub struct CrowdSignal {
    /// How many users are currently in this symbol.
    pub holder_count:   i64,
    /// Average unrealised P&L across all holders (as % of margin).
    pub avg_pnl_pct:    f64,
    /// Total USD margin committed across all holders.
    #[allow(dead_code)]
    pub total_size_usd: f64,
    /// Dominant direction: `"LONG"`, `"SHORT"`, or `"MIXED"`.
    pub crowd_side:     String,
}

impl CrowdSignal {
    /// Confidence multiplier to apply before the entry decision.
    ///
    /// | Scenario                                        | Multiplier |
    /// |-------------------------------------------------|-----------|
    /// | ≥2 users same side, avg P&L > +1.5%            | 1.05–1.12 |
    /// | ≥2 users same side, avg P&L < -1.5%            | 0.82–0.90 |
    /// | ≥2 users opposite side, avg P&L < -1.5%        | 1.05      |
    /// | ≥2 users opposite side, avg P&L > +1.5%        | 0.92      |
    /// | Fewer than 2 users / neutral P&L               | 1.00      |
    ///
    /// The magnitude scales linearly with `holder_count` up to 5 holders.
    pub fn confidence_multiplier(&self, proposed_side: &str) -> f64 {
        if self.holder_count < 2 { return 1.0; }

        let same_direction = self.crowd_side == proposed_side || self.crowd_side == "MIXED";
        let winning = self.avg_pnl_pct >  1.5;
        let losing  = self.avg_pnl_pct < -1.5;
        // Scale effect by number of holders, capped at 5 for maximum effect
        let scale = (self.holder_count as f64 / 5.0).min(1.0);

        match (same_direction, winning, losing) {
            // Crowd is in same direction and winning → confirming signal
            (true, true, _) => 1.0 + 0.12 * scale,
            // Crowd is in same direction and losing → crowd got trapped, caution
            (true, _, true) => 1.0 - 0.18 * scale,
            // Crowd is on the other side and losing → contrarian confirmation
            (false, _, true) => 1.05,
            // Crowd is on the other side and winning → slight caution
            (false, true, _) => 0.92,
            // Inconclusive
            _ => 1.0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Signal alignment encoding
// ─────────────────────────────────────────────────────────────────────────────

/// Encode a signal as +1 (agreed with trade direction), -1 (opposed), or 0 (absent).
fn align(present: bool, bullish: bool, was_long: bool) -> i16 {
    if !present { return 0; }
    if bullish == was_long { 1 } else { -1 }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Trade outcome recording
// ─────────────────────────────────────────────────────────────────────────────

/// Record a closed trade in the collective `trade_outcomes` table.
///
/// Fire-and-forget: errors are logged at DEBUG level but never bubble up
/// to the trading loop — a failed insert must never stall position closes.
pub async fn record_outcome(
    db:         &Database,
    tenant_id:  Option<Uuid>,
    pos:        &PaperPosition,
    exit_price: f64,
    pnl_pct:    f64,
    r_multiple: f64,
) {
    let was_long = pos.side == "LONG";
    let c        = &pos.contrib;

    let outcome = if pnl_pct > 0.5 {
        "win"
    } else if pnl_pct < -0.5 {
        "loss"
    } else {
        "breakeven"
    };

    let _ = sqlx::query!(
        r#"INSERT INTO trade_outcomes (
               tenant_id, symbol, side, entry_price, exit_price, pnl_pct,
               r_multiple, hold_cycles,
               sig_rsi, sig_bollinger, sig_macd, sig_ema_cross, sig_order_flow,
               sig_z_score, sig_volume, sig_sentiment, sig_funding, sig_trend,
               sig_candle, sig_chart,
               outcome
           ) VALUES (
               $1,$2,$3,$4,$5,$6,
               $7,$8,
               $9,$10,$11,$12,$13,
               $14,$15,$16,$17,$18,
               $19,$20,
               $21
           )"#,
        tenant_id,
        pos.symbol,
        pos.side,
        pos.entry_price,
        exit_price,
        pnl_pct,
        r_multiple,
        pos.cycles_held as i32,
        align(true, c.rsi_bullish,       was_long),
        align(true, c.bb_bullish,        was_long),
        align(true, c.macd_bullish,      was_long),
        align(true, c.ema_cross_bullish, was_long),
        align(true, c.of_bullish,        was_long),
        align(c.z_score_present,         c.z_score_bullish,   was_long),
        align(c.volume_present,          c.volume_bullish,    was_long),
        align(c.sentiment_present,       c.sentiment_bullish, was_long),
        align(c.funding_present,         c.funding_bullish,   was_long),
        align(true, c.trend_bullish,     was_long),
        align(c.candle_pattern_present,  c.candle_pattern_bullish, was_long),
        align(c.chart_pattern_present,   c.chart_pattern_bullish,  was_long),
        outcome,
    )
    .execute(db.pool())
    .await
    .map_err(|e| log::debug!("collective: record_outcome failed for {}: {}", pos.symbol, e));
}

// ─────────────────────────────────────────────────────────────────────────────
//  Hot position tracking
// ─────────────────────────────────────────────────────────────────────────────

/// Upsert a newly opened position into `hot_positions`.
/// Called immediately after execute_paper_trade succeeds.
pub async fn upsert_hot_position(
    db:        &Database,
    tenant_id: Uuid,
    pos:       &PaperPosition,
) {
    let _ = sqlx::query!(
        r#"INSERT INTO hot_positions
               (tenant_id, symbol, side, entry_price, size_usd, unrealised_pnl_pct)
           VALUES ($1, $2, $3, $4, $5, 0.0)
           ON CONFLICT (tenant_id, symbol)
           DO UPDATE SET
               entry_price        = EXCLUDED.entry_price,
               size_usd           = EXCLUDED.size_usd,
               unrealised_pnl_pct = 0.0,
               opened_at          = now()"#,
        tenant_id,
        pos.symbol,
        pos.side,
        pos.entry_price,
        pos.size_usd,
    )
    .execute(db.pool())
    .await
    .map_err(|e| log::debug!("collective: upsert_hot_position failed for {}: {}", pos.symbol, e));
}

/// Update the unrealised P&L of a live position.
/// Called in the position management loop each cycle.
pub async fn update_hot_pnl(
    db:         &Database,
    tenant_id:  Uuid,
    symbol:     &str,
    pnl_pct:    f64,
) {
    let _ = sqlx::query!(
        "UPDATE hot_positions SET unrealised_pnl_pct = $1
         WHERE tenant_id = $2 AND symbol = $3",
        pnl_pct,
        tenant_id,
        symbol,
    )
    .execute(db.pool())
    .await
    .map_err(|e| log::debug!("collective: update_hot_pnl failed for {}: {}", symbol, e));
}

/// Delete a position from `hot_positions` when it closes.
/// Called at the top of close_paper_position.
pub async fn remove_hot_position(
    db:        &Database,
    tenant_id: Uuid,
    symbol:    &str,
) {
    let _ = sqlx::query!(
        "DELETE FROM hot_positions WHERE tenant_id = $1 AND symbol = $2",
        tenant_id,
        symbol,
    )
    .execute(db.pool())
    .await
    .map_err(|e| log::debug!("collective: remove_hot_position failed for {}: {}", symbol, e));
}

// ─────────────────────────────────────────────────────────────────────────────
//  Crowd signal query
// ─────────────────────────────────────────────────────────────────────────────

/// Query how many users are currently in `symbol` and how they are performing.
///
/// Returns `None` when no users hold the symbol or the DB is unavailable.
pub async fn get_crowd_signal(db: &Database, symbol: &str) -> Option<CrowdSignal> {
    let row = sqlx::query!(
        r#"SELECT
               COUNT(*)                                       AS holder_count,
               COALESCE(AVG(unrealised_pnl_pct), 0.0)        AS avg_pnl_pct,
               COALESCE(SUM(size_usd), 0.0)                  AS total_size_usd,
               COUNT(CASE WHEN side = 'LONG'  THEN 1 END)    AS longs,
               COUNT(CASE WHEN side = 'SHORT' THEN 1 END)    AS shorts
           FROM hot_positions
           WHERE symbol = $1"#,
        symbol,
    )
    .fetch_optional(db.pool())
    .await
    .ok()??;

    let holder_count = row.holder_count.unwrap_or(0);
    if holder_count == 0 { return None; }

    let longs  = row.longs.unwrap_or(0);
    let shorts = row.shorts.unwrap_or(0);

    let crowd_side = if longs > shorts * 2 {
        "LONG".to_string()
    } else if shorts > longs * 2 {
        "SHORT".to_string()
    } else {
        "MIXED".to_string()
    };

    Some(CrowdSignal {
        holder_count,
        avg_pnl_pct:    row.avg_pnl_pct.unwrap_or(0.0),
        total_size_usd: row.total_size_usd.unwrap_or(0.0),
        crowd_side,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Nightly collective weight recalculation
// ─────────────────────────────────────────────────────────────────────────────

/// Recalculate signal weights from the last 90 days of collective trade outcomes.
///
/// Algorithm:
///   For each signal, compute precision = wins_aligned / (wins_aligned + losses_aligned).
///   A precision above 0.5 means "when this signal agrees with the trade, it tends
///   to win" → weight should increase.  Below 0.5 → weight should decrease.
///
///   The result is blended: 60% collective + 40% current to prevent one bad period
///   from wiping out months of accumulated learning.
///
/// Returns `None` when fewer than `min_trades` outcomes exist (not enough data).
pub async fn recalculate_collective_weights(
    db:              &Database,
    current_weights: &SignalWeights,
    min_trades:      i64,
) -> Option<SignalWeights> {

    // Aggregate signal alignment by win/loss for the past 90 days
    let rows = sqlx::query!(
        r#"SELECT
               outcome,
               SUM(CASE WHEN sig_rsi        =  1 THEN 1 ELSE 0 END) AS rsi_aligned,
               SUM(CASE WHEN sig_rsi        = -1 THEN 1 ELSE 0 END) AS rsi_opposed,
               SUM(CASE WHEN sig_bollinger  =  1 THEN 1 ELSE 0 END) AS bb_aligned,
               SUM(CASE WHEN sig_bollinger  = -1 THEN 1 ELSE 0 END) AS bb_opposed,
               SUM(CASE WHEN sig_macd       =  1 THEN 1 ELSE 0 END) AS macd_aligned,
               SUM(CASE WHEN sig_macd       = -1 THEN 1 ELSE 0 END) AS macd_opposed,
               SUM(CASE WHEN sig_ema_cross  =  1 THEN 1 ELSE 0 END) AS ema_aligned,
               SUM(CASE WHEN sig_ema_cross  = -1 THEN 1 ELSE 0 END) AS ema_opposed,
               SUM(CASE WHEN sig_order_flow =  1 THEN 1 ELSE 0 END) AS of_aligned,
               SUM(CASE WHEN sig_order_flow = -1 THEN 1 ELSE 0 END) AS of_opposed,
               SUM(CASE WHEN sig_z_score    =  1 THEN 1 ELSE 0 END) AS z_aligned,
               SUM(CASE WHEN sig_z_score    = -1 THEN 1 ELSE 0 END) AS z_opposed,
               SUM(CASE WHEN sig_volume     =  1 THEN 1 ELSE 0 END) AS vol_aligned,
               SUM(CASE WHEN sig_volume     = -1 THEN 1 ELSE 0 END) AS vol_opposed,
               SUM(CASE WHEN sig_sentiment  =  1 THEN 1 ELSE 0 END) AS sent_aligned,
               SUM(CASE WHEN sig_sentiment  = -1 THEN 1 ELSE 0 END) AS sent_opposed,
               SUM(CASE WHEN sig_funding    =  1 THEN 1 ELSE 0 END) AS fund_aligned,
               SUM(CASE WHEN sig_funding    = -1 THEN 1 ELSE 0 END) AS fund_opposed,
               SUM(CASE WHEN sig_trend      =  1 THEN 1 ELSE 0 END) AS trend_aligned,
               SUM(CASE WHEN sig_trend      = -1 THEN 1 ELSE 0 END) AS trend_opposed,
               SUM(CASE WHEN sig_candle     =  1 THEN 1 ELSE 0 END) AS cnd_aligned,
               SUM(CASE WHEN sig_candle     = -1 THEN 1 ELSE 0 END) AS cnd_opposed,
               SUM(CASE WHEN sig_chart      =  1 THEN 1 ELSE 0 END) AS cht_aligned,
               SUM(CASE WHEN sig_chart      = -1 THEN 1 ELSE 0 END) AS cht_opposed,
               COUNT(*) AS trade_count
           FROM trade_outcomes
           WHERE closed_at > now() - INTERVAL '90 days'
           GROUP BY outcome"#
    )
    .fetch_all(db.pool())
    .await
    .ok()?;

    let total: i64 = rows.iter().map(|r| r.trade_count.unwrap_or(0)).sum();
    if total < min_trades {
        log::info!(
            "🧠 Collective: {} outcomes recorded — need {} before recalculating weights",
            total, min_trades
        );
        return None;
    }

    // Accumulate win/total counts per signal
    macro_rules! tally {
        ($rows:expr, $wins:ident, $tot:ident, $aligned:ident, $opposed:ident) => {
            let mut $wins = 0i64;
            let mut $tot  = 0i64;
            for row in &$rows {
                let a = row.$aligned.unwrap_or(0);
                let o = row.$opposed.unwrap_or(0);
                $tot += a + o;
                if row.outcome == "win" { $wins += a; }
            }
        };
    }

    tally!(rows, rsi_w,  rsi_t,  rsi_aligned,  rsi_opposed);
    tally!(rows, bb_w,   bb_t,   bb_aligned,   bb_opposed);
    tally!(rows, macd_w, macd_t, macd_aligned, macd_opposed);
    tally!(rows, ema_w,  ema_t,  ema_aligned,  ema_opposed);
    tally!(rows, of_w,   of_t,   of_aligned,   of_opposed);
    tally!(rows, z_w,    z_t,    z_aligned,    z_opposed);
    tally!(rows, vol_w,  vol_t,  vol_aligned,  vol_opposed);
    tally!(rows, sent_w, sent_t, sent_aligned, sent_opposed);
    tally!(rows, fund_w, fund_t, fund_aligned, fund_opposed);
    tally!(rows, trend_w,trend_t,trend_aligned,trend_opposed);
    tally!(rows, cnd_w,  cnd_t,  cnd_aligned,  cnd_opposed);
    tally!(rows, cht_w,  cht_t,  cht_aligned,  cht_opposed);

    // Laplace-smoothed precision: P(win | aligned) with add-1/add-2 smoothing
    fn precision(wins: i64, total: i64) -> f64 {
        (wins as f64 + 1.0) / (total as f64 + 2.0)
    }

    // Map precision to a weight adjustment.
    // Precision 0.5 = 50/50 = no information → keep current weight.
    // Precision 1.0 = signal always right → scale weight up.
    // The scaling is: collective_weight = current_weight × (precision / 0.5)
    //   i.e. precision 0.6 → multiply by 1.2; precision 0.4 → multiply by 0.8.
    // Then blend 60% collective + 40% current.
    fn blend(precision: f64, current: f64) -> f64 {
        let scaled = current * (precision / 0.5);
        scaled * 0.60 + current * 0.40
    }

    let mut new_w = current_weights.clone();
    new_w.rsi          = blend(precision(rsi_w,  rsi_t),  current_weights.rsi);
    new_w.bollinger    = blend(precision(bb_w,   bb_t),   current_weights.bollinger);
    new_w.macd         = blend(precision(macd_w, macd_t), current_weights.macd);
    new_w.ema_cross    = blend(precision(ema_w,  ema_t),  current_weights.ema_cross);
    new_w.order_flow   = blend(precision(of_w,   of_t),   current_weights.order_flow);
    new_w.z_score      = blend(precision(z_w,    z_t),    current_weights.z_score);
    new_w.volume       = blend(precision(vol_w,  vol_t),  current_weights.volume);
    new_w.sentiment    = blend(precision(sent_w, sent_t), current_weights.sentiment);
    new_w.funding_rate = blend(precision(fund_w, fund_t), current_weights.funding_rate);
    new_w.trend        = blend(precision(trend_w,trend_t),current_weights.trend);
    // candle_pattern and chart_pattern if they exist in SignalWeights
    // (they were added in a prior session — blend them too)
    new_w.candle_pattern = blend(precision(cnd_w, cnd_t), current_weights.candle_pattern);
    new_w.chart_pattern  = blend(precision(cht_w, cht_t), current_weights.chart_pattern);

    new_w.clamp_and_normalise();

    log::info!(
        "🧠 Collective weights recalculated from {} trades (90d) → \
         rsi:{:.3} bb:{:.3} macd:{:.3} ema:{:.3} of:{:.3} z:{:.3} vol:{:.3}",
        total,
        new_w.rsi, new_w.bollinger, new_w.macd, new_w.ema_cross,
        new_w.order_flow, new_w.z_score, new_w.volume
    );

    Some(new_w)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crowd_multiplier_returns_1_for_single_holder() {
        let s = CrowdSignal {
            holder_count: 1, avg_pnl_pct: 10.0,
            total_size_usd: 500.0, crowd_side: "LONG".to_string(),
        };
        assert_eq!(s.confidence_multiplier("LONG"), 1.0);
    }

    #[test]
    fn crowd_winning_same_side_increases_confidence() {
        let s = CrowdSignal {
            holder_count: 5, avg_pnl_pct: 5.0,
            total_size_usd: 2500.0, crowd_side: "LONG".to_string(),
        };
        let mult = s.confidence_multiplier("LONG");
        assert!(mult > 1.0, "winning crowd same side should increase confidence, got {}", mult);
        assert!(mult <= 1.15, "multiplier capped, got {}", mult);
    }

    #[test]
    fn crowd_losing_same_side_decreases_confidence() {
        let s = CrowdSignal {
            holder_count: 5, avg_pnl_pct: -4.0,
            total_size_usd: 2500.0, crowd_side: "LONG".to_string(),
        };
        let mult = s.confidence_multiplier("LONG");
        assert!(mult < 1.0, "losing crowd same side should decrease confidence, got {}", mult);
        assert!(mult >= 0.80, "floor respected, got {}", mult);
    }

    #[test]
    fn crowd_losing_opposite_side_is_contrarian_boost() {
        // Others shorted and are losing → we want to go LONG (contrarian edge)
        let s = CrowdSignal {
            holder_count: 3, avg_pnl_pct: -3.0,
            total_size_usd: 1500.0, crowd_side: "SHORT".to_string(),
        };
        let mult = s.confidence_multiplier("LONG");
        assert!(mult > 1.0, "contrarian signal should boost confidence, got {}", mult);
    }

    #[test]
    fn crowd_winning_opposite_side_is_caution() {
        // Others shorted and are winning → our LONG is risky
        let s = CrowdSignal {
            holder_count: 4, avg_pnl_pct: 3.5,
            total_size_usd: 2000.0, crowd_side: "SHORT".to_string(),
        };
        let mult = s.confidence_multiplier("LONG");
        assert!(mult < 1.0, "opposing winning crowd should decrease confidence, got {}", mult);
    }

    #[test]
    fn neutral_pnl_returns_1_multiplier() {
        let s = CrowdSignal {
            holder_count: 3, avg_pnl_pct: 0.2, // below both thresholds
            total_size_usd: 1500.0, crowd_side: "LONG".to_string(),
        };
        assert_eq!(s.confidence_multiplier("LONG"), 1.0);
    }

    #[test]
    fn align_encoding_correct() {
        assert_eq!(align(false, true,  true),  0,  "absent signal → 0");
        assert_eq!(align(true,  true,  true),  1,  "bullish + long = aligned → +1");
        assert_eq!(align(true,  false, false),  1, "bearish + short = aligned → +1");
        assert_eq!(align(true,  false, true),  -1, "bearish + long = opposed → -1");
        assert_eq!(align(true,  true,  false), -1, "bullish + short = opposed → -1");
    }
}
