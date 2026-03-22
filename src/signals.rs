//! Order-book signal extraction.
//!
//! ## What we compute
//!
//! ### Whole-book imbalance
//! Simple bid/ask volume ratio across the top-20 levels.  A 3:1 ratio means
//! buyers are absorbing 3× more supply than sellers — historically bullish.
//!
//! ### Near-price depth (within 0.5% of mid)
//! The first 0.5% of depth on each side is the most price-sensitive zone.
//! An imbalance HERE is a stronger signal than the total-book imbalance because
//! market orders hit this depth first.  We weight this zone 2× in the final
//! confidence score.
//!
//! ### Bid/ask walls
//! A "wall" is any single level containing ≥15% of its side's total depth AND
//! ≥3× the median level size on that side.  Walls act as support (bid wall)
//! and resistance (ask wall) and directly affect momentum probability:
//!   - Bid wall below price → bullish (buyers defending the level)
//!   - Ask wall above price → bearish (sellers capping the move)
//!
//! ### Spread
//! Wide spreads signal uncertainty and low liquidity.  We record the best-bid
//! / best-ask spread as a percentage of mid so callers can gate signals.
//!
//! ### Market sentiment label
//! A composite label `STRONGLY_BULLISH | BULLISH | NEUTRAL | BEARISH |
//! STRONGLY_BEARISH` combining all of the above.  Used by the decision engine
//! to confirm or override other signals when sentiment is extreme.

use crate::data::OrderBook;
use anyhow::Result;
use serde::{Deserialize, Serialize};

// ─────────────────────────── Output structs ──────────────────────────────────

/// A single order-book price wall (large resting order).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookWall {
    /// Price level of the wall.
    pub price: f64,
    /// Total quantity at this level.
    pub volume: f64,
    /// Fraction of the side's total depth this level represents (0–1).
    pub depth_frac: f64,
    /// True = bid wall (support), False = ask wall (resistance).
    pub is_bid: bool,
}

/// Enriched order-flow signal derived from the full L2 book.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrderFlowSignal {
    // ── Whole-book metrics ────────────────────────────────────────────────
    pub bid_volume: f64,
    pub ask_volume: f64,
    /// bid_volume / ask_volume  (0 → ∞; 1.0 = perfectly balanced).
    pub imbalance_ratio: f64,

    // ── Near-price depth (within 0.5 % of mid) ───────────────────────────
    /// Bid volume within 0.5% of mid price.
    pub near_bid_vol: f64,
    /// Ask volume within 0.5% of mid price.
    pub near_ask_vol: f64,
    /// near_bid_vol / near_ask_vol.
    pub near_imbalance: f64,

    // ── Spread ────────────────────────────────────────────────────────────
    /// (best_ask - best_bid) / mid_price × 100.  0.0 if book is empty.
    pub spread_pct: f64,

    // ── Walls ─────────────────────────────────────────────────────────────
    /// Up to 3 significant walls found across both sides, sorted by depth_frac desc.
    pub walls: Vec<BookWall>,
    /// True if there is a bid wall within 2% below mid price.
    pub bid_wall_near: bool,
    /// True if there is an ask wall within 2% above mid price.
    pub ask_wall_near: bool,

    // ── Summary ───────────────────────────────────────────────────────────
    /// "LONG" | "SHORT" | "NEUTRAL"
    pub direction: String,
    /// 0.50 – 0.95 composite confidence.
    pub confidence: f64,
    /// Human-readable label for the AI reviewer and dashboard.
    pub sentiment: String,
}

// ─────────────────────────── Detection logic ─────────────────────────────────

/// Minimum fraction of side total to qualify as a wall.
const WALL_DEPTH_FRAC: f64 = 0.15;
/// Minimum multiple of the median level size to qualify as a wall.
const WALL_MEDIAN_MULT: f64 = 3.0;
/// Depth zone radius as a fraction of mid price.
const NEAR_ZONE_PCT: f64 = 0.005; // 0.5 %

pub fn detect_order_flow(orderbook: &OrderBook) -> Result<OrderFlowSignal> {
    // ── Whole-book volumes ────────────────────────────────────────────────
    let bid_volume: f64 = orderbook.bids.iter().map(|(_, v)| v).sum();
    let ask_volume: f64 = orderbook.asks.iter().map(|(_, v)| v).sum();

    let imbalance_ratio = if ask_volume > 1e-9 {
        bid_volume / ask_volume
    } else {
        1.0
    };

    // ── Mid price ─────────────────────────────────────────────────────────
    let best_bid = orderbook.bids.first().map(|(p, _)| *p).unwrap_or(0.0);
    let best_ask = orderbook.asks.first().map(|(p, _)| *p).unwrap_or(0.0);
    let mid = if best_bid > 0.0 && best_ask > 0.0 {
        (best_bid + best_ask) / 2.0
    } else if best_bid > 0.0 {
        best_bid
    } else if best_ask > 0.0 {
        best_ask
    } else {
        0.0
    };

    // ── Spread ────────────────────────────────────────────────────────────
    let spread_pct = if mid > 1e-9 {
        (best_ask - best_bid) / mid * 100.0
    } else {
        0.0
    };

    // ── Near-price depth ─────────────────────────────────────────────────
    let near_bid_vol: f64 = if mid > 0.0 {
        orderbook
            .bids
            .iter()
            .filter(|(p, _)| (mid - p) / mid <= NEAR_ZONE_PCT)
            .map(|(_, v)| v)
            .sum()
    } else {
        0.0
    };
    let near_ask_vol: f64 = if mid > 0.0 {
        orderbook
            .asks
            .iter()
            .filter(|(p, _)| (p - mid) / mid <= NEAR_ZONE_PCT)
            .map(|(_, v)| v)
            .sum()
    } else {
        0.0
    };
    let near_imbalance = if near_ask_vol > 1e-9 {
        near_bid_vol / near_ask_vol
    } else {
        1.0
    };

    // ── Wall detection ────────────────────────────────────────────────────
    fn find_walls(levels: &[(f64, f64)], total: f64, is_bid: bool) -> Vec<BookWall> {
        if levels.is_empty() || total < 1e-9 {
            return vec![];
        }
        // Median of level sizes
        let mut sizes: Vec<f64> = levels.iter().map(|(_, v)| *v).collect();
        sizes.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sizes[sizes.len() / 2];

        levels
            .iter()
            .filter(|(_, v)| {
                let frac = v / total;
                frac >= WALL_DEPTH_FRAC && *v >= median * WALL_MEDIAN_MULT
            })
            .map(|(p, v)| BookWall {
                price: *p,
                volume: *v,
                depth_frac: v / total,
                is_bid,
            })
            .collect()
    }

    let mut bid_walls = find_walls(&orderbook.bids, bid_volume, true);
    let mut ask_walls = find_walls(&orderbook.asks, ask_volume, false);

    // Keep top-3 walls by depth_frac
    bid_walls.sort_by(|a, b| b.depth_frac.partial_cmp(&a.depth_frac).unwrap());
    ask_walls.sort_by(|a, b| b.depth_frac.partial_cmp(&a.depth_frac).unwrap());

    let bid_wall_near = bid_walls.iter().any(|w| {
        mid > 0.0 && (mid - w.price) / mid <= 0.02 // within 2% below mid
    });
    let ask_wall_near = ask_walls.iter().any(|w| {
        mid > 0.0 && (w.price - mid) / mid <= 0.02 // within 2% above mid
    });

    let mut walls: Vec<BookWall> = bid_walls.into_iter().chain(ask_walls).collect();
    walls.sort_by(|a, b| b.depth_frac.partial_cmp(&a.depth_frac).unwrap());
    walls.truncate(4);

    // ── Composite direction & confidence ─────────────────────────────────
    // Score combines whole-book and near-price imbalances.
    // Near-price counts double: it's where market orders hit first.
    let composite = (imbalance_ratio + 2.0 * near_imbalance) / 3.0;

    let direction = if composite > 1.5 {
        "LONG".to_string()
    } else if composite < 0.67 {
        "SHORT".to_string()
    } else {
        "NEUTRAL".to_string()
    };

    // Confidence tiers (symmetric for LONG/SHORT)
    let base_conf = match composite {
        r if r > 3.0 => 0.95,
        r if r > 2.0 => 0.85,
        r if r > 1.5 => 0.70,
        r if r < 0.33 => 0.95,
        r if r < 0.50 => 0.85,
        r if r < 0.67 => 0.70,
        _ => 0.50,
    };

    // Wall modifier: bid wall near price when LONG → +0.05, ask wall near when SHORT → +0.05
    // Opposing wall → −0.05 (e.g. strong ask wall capping a LONG signal)
    let wall_bonus: f64 = match direction.as_str() {
        "LONG" => {
            if bid_wall_near {
                0.05
            } else if ask_wall_near {
                -0.05
            } else {
                0.0
            }
        }
        "SHORT" => {
            if ask_wall_near {
                0.05
            } else if bid_wall_near {
                -0.05
            } else {
                0.0
            }
        }
        _ => 0.0,
    };

    let confidence = (base_conf + wall_bonus).clamp(0.45, 0.98);

    // ── Sentiment label ───────────────────────────────────────────────────
    let sentiment = match (direction.as_str(), confidence) {
        ("LONG", c) if c >= 0.90 => "STRONGLY_BULLISH",
        ("LONG", _) => "BULLISH",
        ("SHORT", c) if c >= 0.90 => "STRONGLY_BEARISH",
        ("SHORT", _) => "BEARISH",
        _ => "NEUTRAL",
    }
    .to_string();

    Ok(OrderFlowSignal {
        bid_volume,
        ask_volume,
        imbalance_ratio,
        near_bid_vol,
        near_ask_vol,
        near_imbalance,
        spread_pct,
        walls,
        bid_wall_near,
        ask_wall_near,
        direction,
        confidence,
        sentiment,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
//  UNIT TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::OrderBook;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn book(bid_vol: f64, ask_vol: f64) -> OrderBook {
        // Single level each side — enough to drive imbalance_ratio
        OrderBook {
            symbol: "TEST".to_string(),
            timestamp: 0,
            bids: if bid_vol > 0.0 {
                vec![(100.0, bid_vol)]
            } else {
                vec![]
            },
            asks: if ask_vol > 0.0 {
                vec![(100.1, ask_vol)]
            } else {
                vec![]
            },
        }
    }

    fn multi_level_book(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> OrderBook {
        OrderBook {
            symbol: "TEST".to_string(),
            timestamp: 0,
            bids: bids.to_vec(),
            asks: asks.to_vec(),
        }
    }

    // ── Direction ────────────────────────────────────────────────────────────

    #[test]
    fn direction_long_when_bids_dominate() {
        let sig = detect_order_flow(&book(300.0, 100.0)).unwrap();
        assert_eq!(sig.direction, "LONG", "3:1 bid:ask should be LONG");
    }

    #[test]
    fn direction_short_when_asks_dominate() {
        let sig = detect_order_flow(&book(100.0, 300.0)).unwrap();
        assert_eq!(sig.direction, "SHORT", "1:3 bid:ask should be SHORT");
    }

    #[test]
    fn direction_neutral_when_balanced() {
        let sig = detect_order_flow(&book(100.0, 100.0)).unwrap();
        assert_eq!(sig.direction, "NEUTRAL", "1:1 should be NEUTRAL");
    }

    #[test]
    fn direction_neutral_boundary_just_above_threshold() {
        // imbalance = 1.5 exactly hits the > 1.5 boundary → NOT long (needs > 1.5)
        let sig = detect_order_flow(&book(150.0, 100.0)).unwrap();
        // ratio = 1.5, which is NOT > 1.5, so should be NEUTRAL
        assert_eq!(
            sig.direction, "NEUTRAL",
            "ratio exactly 1.5 is the threshold boundary — should be NEUTRAL"
        );
    }

    #[test]
    fn direction_long_just_above_threshold() {
        let sig = detect_order_flow(&book(151.0, 100.0)).unwrap();
        assert_eq!(sig.direction, "LONG", "151:100 = 1.51 ratio should be LONG");
    }

    // ── Confidence LONG ───────────────────────────────────────────────────────

    #[test]
    fn confidence_long_tier_1_above_3x() {
        let sig = detect_order_flow(&book(400.0, 100.0)).unwrap();
        assert_eq!(
            sig.confidence, 0.95,
            "4:1 bid:ask should yield 0.95 confidence"
        );
    }

    #[test]
    fn confidence_long_tier_2_above_2x() {
        let sig = detect_order_flow(&book(250.0, 100.0)).unwrap();
        assert_eq!(
            sig.confidence, 0.85,
            "2.5:1 bid:ask should yield 0.85 confidence"
        );
    }

    #[test]
    fn confidence_long_tier_3_above_1_5x() {
        let sig = detect_order_flow(&book(180.0, 100.0)).unwrap();
        assert_eq!(
            sig.confidence, 0.70,
            "1.8:1 bid:ask should yield 0.70 confidence"
        );
    }

    // ── Confidence SHORT — REGRESSION (previously all returned 0.50) ──────────

    #[test]
    fn confidence_short_tier_1_asks_3x_bids_regression() {
        // Pre-fix: imbalance_ratio = 0.25 hit the `_ => 0.50` arm.
        // Post-fix: should return 0.95 (symmetric with LONG tier 1).
        let sig = detect_order_flow(&book(100.0, 400.0)).unwrap();
        assert_eq!(sig.direction, "SHORT");
        assert_eq!(
            sig.confidence, 0.95,
            "REGRESSION: 1:4 ask:bid (ratio=0.25) should be 0.95, was 0.50 before fix"
        );
    }

    #[test]
    fn confidence_short_tier_2_asks_2x_bids_regression() {
        let sig = detect_order_flow(&book(100.0, 250.0)).unwrap();
        assert_eq!(sig.direction, "SHORT");
        assert_eq!(
            sig.confidence, 0.85,
            "REGRESSION: 1:2.5 ask:bid (ratio≈0.40) should be 0.85, was 0.50 before fix"
        );
    }

    #[test]
    fn confidence_short_tier_3_asks_1_5x_bids_regression() {
        let sig = detect_order_flow(&book(100.0, 180.0)).unwrap();
        assert_eq!(sig.direction, "SHORT");
        assert_eq!(
            sig.confidence, 0.70,
            "REGRESSION: 1:1.8 ask:bid (ratio≈0.56) should be 0.70, was 0.50 before fix"
        );
    }

    #[test]
    fn confidence_short_symmetry_with_long() {
        // Long at 4:1 and Short at 1:4 should have identical confidence
        let long_sig = detect_order_flow(&book(400.0, 100.0)).unwrap();
        let short_sig = detect_order_flow(&book(100.0, 400.0)).unwrap();
        assert_eq!(
            long_sig.confidence, short_sig.confidence,
            "Long (4:1) and Short (1:4) must have symmetric confidence"
        );
    }

    #[test]
    fn confidence_neutral_returns_050() {
        let sig = detect_order_flow(&book(120.0, 100.0)).unwrap();
        // ratio = 1.2, inside the NEUTRAL zone
        assert_eq!(sig.direction, "NEUTRAL");
        assert_eq!(sig.confidence, 0.50, "neutral zone should always be 0.50");
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn empty_asks_gives_neutral_imbalance() {
        let b = multi_level_book(&[(100.0, 50.0)], &[]);
        let sig = detect_order_flow(&b).unwrap();
        // ask_volume = 0 → imbalance fallback to 1.0 → NEUTRAL
        assert_eq!(
            sig.imbalance_ratio, 1.0,
            "zero asks should produce 1.0 ratio"
        );
        assert_eq!(sig.direction, "NEUTRAL");
    }

    #[test]
    fn empty_book_gives_neutral() {
        let b = multi_level_book(&[], &[]);
        let sig = detect_order_flow(&b).unwrap();
        assert_eq!(sig.direction, "NEUTRAL", "empty book should be NEUTRAL");
        assert_eq!(
            sig.confidence, 0.50,
            "empty book should have 0.50 confidence"
        );
    }

    #[test]
    fn multi_level_volumes_summed_correctly() {
        // Book with 3:1 whole-book imbalance AND strong near-price imbalance.
        //
        // Levels (best bid 100.0, best ask 100.1 → mid = 100.05):
        //   bids: (100.0, 250) — 0.05/100.05 = 0.05% < 0.5% ✓ near   bid_total = 300
        //         ( 99.0,  50) — 1.05/100.05 = 1.05% > 0.5% ✗ far
        //   asks: (100.1,  60) — 0.05/100.05 = 0.05% < 0.5% ✓ near   ask_total = 100
        //         (101.5,  40) — 1.45/100.05 = 1.45% > 0.5% ✗ far
        //
        // near_bid = 250, near_ask = 60 → near_imbalance = 4.17
        // composite = (3.0 + 2 × 4.17) / 3 = 3.78 → base_conf = 0.95 (> 3.0 tier)
        let b = multi_level_book(
            &[(100.0, 250.0), (99.0, 50.0)],
            &[(100.1, 60.0), (101.5, 40.0)],
        );
        let sig = detect_order_flow(&b).unwrap();
        assert_eq!(sig.bid_volume, 300.0);
        assert_eq!(sig.ask_volume, 100.0);
        assert_eq!(
            sig.direction, "LONG",
            "3:1 bid:ask with deep near-side should be LONG"
        );
        // near-price imbalance (250:60 ≈ 4.2) pushes composite to 3.78 → confidence ≥ 0.85
        assert!(
            sig.confidence >= 0.85,
            "confidence={} should be ≥ 0.85",
            sig.confidence
        );
    }

    #[test]
    fn imbalance_ratio_computed_correctly() {
        let sig = detect_order_flow(&book(200.0, 80.0)).unwrap();
        let expected = 200.0 / 80.0;
        assert!(
            (sig.imbalance_ratio - expected).abs() < 1e-10,
            "imbalance_ratio should be bid/ask, got {}",
            sig.imbalance_ratio
        );
    }

    // ── Sentiment labels ──────────────────────────────────────────────────────

    #[test]
    fn sentiment_strongly_bullish_on_dominant_bids() {
        let sig = detect_order_flow(&book(400.0, 100.0)).unwrap();
        assert_eq!(sig.sentiment, "STRONGLY_BULLISH");
    }

    #[test]
    fn sentiment_strongly_bearish_on_dominant_asks() {
        let sig = detect_order_flow(&book(100.0, 400.0)).unwrap();
        assert_eq!(sig.sentiment, "STRONGLY_BEARISH");
    }

    #[test]
    fn sentiment_neutral_on_balanced_book() {
        let sig = detect_order_flow(&book(100.0, 100.0)).unwrap();
        assert_eq!(sig.sentiment, "NEUTRAL");
    }

    // ── Spread calculation ────────────────────────────────────────────────────

    #[test]
    fn spread_pct_computed_from_best_bid_ask() {
        // best_bid=100.0, best_ask=100.2, mid=100.1, spread=0.2/100.1*100≈0.20%
        let sig = detect_order_flow(&book(100.0, 100.0)).unwrap();
        // book() places bid at 100.0 and ask at 100.1, spread = 0.1/100.05*100 ≈ 0.10%
        assert!(sig.spread_pct >= 0.0, "spread_pct must be non-negative");
        assert!(
            sig.spread_pct < 1.0,
            "spread_pct should be <1% for tight market"
        );
    }

    #[test]
    fn spread_pct_zero_when_no_book() {
        let b = multi_level_book(&[], &[]);
        let sig = detect_order_flow(&b).unwrap();
        assert_eq!(sig.spread_pct, 0.0, "empty book should have 0.0 spread");
    }

    // ── Near-price depth ─────────────────────────────────────────────────────

    #[test]
    fn near_price_depth_excludes_far_levels() {
        // mid ≈ 100.05; near zone = within 0.5% = [99.55, 100.55]
        // bid at 100.0 (in), bid at 95.0 (far out)
        // ask at 100.1 (in), ask at 110.0 (far out)
        let b = multi_level_book(
            &[(100.0, 80.0), (95.0, 200.0)],
            &[(100.1, 60.0), (110.0, 500.0)],
        );
        let sig = detect_order_flow(&b).unwrap();
        // near_bid_vol should only include the 100.0 level
        assert!(
            (sig.near_bid_vol - 80.0).abs() < 1e-6,
            "near_bid_vol should be 80, got {}",
            sig.near_bid_vol
        );
        // near_ask_vol should only include the 100.1 level
        assert!(
            (sig.near_ask_vol - 60.0).abs() < 1e-6,
            "near_ask_vol should be 60, got {}",
            sig.near_ask_vol
        );
    }

    // ── Wall detection ────────────────────────────────────────────────────────

    #[test]
    fn bid_wall_detected_when_one_level_dominates() {
        // One huge bid level at 99.5 (180/300 = 60% of total, well above 15% threshold)
        // and two small levels: 60 + 60 = 120.  Median of [60,60,180]=60; wall requires ≥3×60=180.
        let b = multi_level_book(
            &[(100.0, 60.0), (99.8, 60.0), (99.5, 180.0)],
            &[(100.2, 100.0)],
        );
        let sig = detect_order_flow(&b).unwrap();
        assert!(!sig.walls.is_empty(), "should detect a wall");
        let wall = sig
            .walls
            .iter()
            .find(|w| w.is_bid)
            .expect("should have a bid wall");
        assert!(
            (wall.price - 99.5).abs() < 1e-6,
            "wall price should be 99.5"
        );
        assert!(wall.depth_frac >= 0.15, "wall depth_frac should be ≥15%");
    }

    #[test]
    fn ask_wall_detected_when_ask_level_dominates() {
        // One huge ask at 101.5: 180/(60+60+180)=60%. Median of asks=[60,60,180]=60; 180≥3×60.
        let b = multi_level_book(
            &[(100.0, 100.0)],
            &[(100.2, 60.0), (101.0, 60.0), (101.5, 180.0)],
        );
        let sig = detect_order_flow(&b).unwrap();
        let wall = sig
            .walls
            .iter()
            .find(|w| !w.is_bid)
            .expect("should have an ask wall");
        assert!(
            (wall.price - 101.5).abs() < 1e-6,
            "wall price should be 101.5"
        );
    }

    #[test]
    fn no_wall_when_book_is_uniform() {
        // All levels equal size — no single level ≥ 3× median
        let b = multi_level_book(
            &[(100.0, 50.0), (99.9, 50.0), (99.8, 50.0)],
            &[(100.1, 50.0), (100.2, 50.0), (100.3, 50.0)],
        );
        let sig = detect_order_flow(&b).unwrap();
        assert!(sig.walls.is_empty(), "uniform book should have no walls");
    }

    #[test]
    fn bid_wall_near_flag_set_within_2pct() {
        // mid ≈ 100.05; a bid wall 1% below mid = 99.05 → within 2% → bid_wall_near = true
        // 200 units at 99.0 vs 3×30=90: 200 ≥ 90 and 200/260 ≈ 77% ≥ 15%
        let b = multi_level_book(
            &[(100.0, 30.0), (99.5, 30.0), (99.0, 200.0)],
            &[(100.1, 100.0)],
        );
        let sig = detect_order_flow(&b).unwrap();
        // wall at 99.0: (100.05 - 99.0) / 100.05 ≈ 1.05% < 2% → near
        assert!(
            sig.bid_wall_near,
            "bid wall at 99.0 should be within 2% of mid≈100.05"
        );
    }
}
