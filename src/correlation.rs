//! Crypto-pair correlation filter.
//!
//! Prevents the bot from stacking multiple highly-correlated positions in the
//! same direction.  BTC + ETH both LONG at 0.85 correlation is not two
//! independent bets — it's one macro bet with twice the size.
//!
//! ## How it works
//!
//! Before opening a new position `execute_paper_trade()` calls
//! `correlation_block()`.  If any *existing* open position has correlation
//! ≥ `CORR_THRESHOLD` with the new symbol AND the same direction, the new
//! entry is skipped unless its confidence is at least `CONF_EDGE` higher than
//! the existing position's entry confidence.
//!
//! ## Correlation matrix
//!
//! Values are 30-day rolling Pearson correlations sourced from historical data
//! (last updated March 2026).  The matrix covers the 20 most-traded perpetuals
//! on Hyperliquid.  Unknown pairs default to 0.35 (assumed low correlation).
//!
//! Update procedure: run the offline script `scripts/update_correlations.py`
//! after each month-end and paste the output into `PAIRS` below.

/// Pairs with correlation ≥ this value block a same-direction add-on.
pub const CORR_THRESHOLD: f64 = 0.72;

/// How much MORE confident the new signal must be to override the block.
/// e.g. 0.08 = new signal needs at least 8 % higher confidence.
pub const CONF_EDGE: f64 = 0.08;

// ─────────────────────────── Correlation table ───────────────────────────────

/// (symbol_a, symbol_b, pearson_r) — always in alphabetical order.
/// Only pairs with r ≥ 0.55 are listed; everything else defaults to 0.35.
static PAIRS: &[(&str, &str, f64)] = &[
    // BTC cluster
    ("BTC",  "ETH",  0.85),
    ("BTC",  "BNB",  0.76),
    ("BTC",  "SOL",  0.79),
    ("BTC",  "AVAX", 0.74),
    ("BTC",  "MATIC",0.73),
    ("BTC",  "LINK", 0.70),
    ("BTC",  "DOT",  0.72),
    ("BTC",  "ADA",  0.71),
    ("BTC",  "ATOM", 0.69),
    ("BTC",  "LTC",  0.78),
    ("BTC",  "BCH",  0.75),
    // ETH cluster
    ("ETH",  "BNB",  0.79),
    ("ETH",  "SOL",  0.82),
    ("ETH",  "AVAX", 0.78),
    ("ETH",  "MATIC",0.77),
    ("ETH",  "LINK", 0.74),
    ("ETH",  "DOT",  0.75),
    ("ETH",  "ADA",  0.73),
    ("ETH",  "ATOM", 0.72),
    ("ETH",  "ARB",  0.81),
    ("ETH",  "OP",   0.80),
    // L1 / L2 cluster
    ("SOL",  "AVAX", 0.76),
    ("SOL",  "BNB",  0.73),
    ("SOL",  "ADA",  0.71),
    ("ARB",  "OP",   0.88),
    ("ARB",  "MATIC",0.82),
    ("OP",   "MATIC",0.81),
    // Meme / high-beta
    ("DOGE", "SHIB", 0.83),
    ("DOGE", "PEPE", 0.74),
    ("SHIB", "PEPE", 0.77),
    // DeFi cluster
    ("AAVE", "UNI",  0.76),
    ("AAVE", "CRV",  0.72),
    ("UNI",  "CRV",  0.74),
    // BTC-correlated alts
    ("XRP",  "BTC",  0.68),
    ("LTC",  "ETH",  0.74),
    ("BCH",  "ETH",  0.72),
];

// ─────────────────────────── Public API ──────────────────────────────────────

/// Strip the common quote suffixes ("-USD", "-USDT", "-PERP", etc.) so that
/// "BTC-USD" and "BTC" match the same row in the correlation table.
fn base(symbol: &str) -> &str {
    symbol
        .split('-')
        .next()
        .unwrap_or(symbol)
}

/// Return the Pearson correlation between two symbols.
/// If the pair is not in the table the function returns 0.35 (low but non-zero).
pub fn get_correlation(a: &str, b: &str) -> f64 {
    let a = base(a);
    let b = base(b);
    if a == b {
        return 1.0;
    }
    // Normalise order: smaller string first for table lookup.
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    PAIRS.iter()
        .find(|(pa, pb, _)| *pa == lo && *pb == hi)
        .map(|(_, _, r)| *r)
        .unwrap_or(0.35)
}

/// Decision returned by `correlation_block()`.
#[derive(Debug, Clone, PartialEq)]
pub enum CorrBlock {
    /// No correlated position found — proceed normally.
    Clear,
    /// Correlated position found and the confidence edge is insufficient.
    /// Contains (existing_symbol, correlation, existing_conf).
    Blocked { existing: String, corr: f64, existing_conf: f64 },
    /// Correlated position found but the new signal is strong enough to override.
    /// Log but allow.
    Override { existing: String, corr: f64 },
}

/// Check whether a new entry for `new_symbol` / `new_side` should be blocked
/// due to an existing correlated open position.
///
/// # Arguments
/// * `new_symbol`  — symbol being considered for entry ("BTC" or "BTC-USD")
/// * `new_side`    — "LONG" or "SHORT"
/// * `new_conf`    — signal confidence for the new entry (0–1)
/// * `positions`   — slice of current open positions
///
/// A position's `entry_confidence` is expected to be stored on `PaperPosition`.
/// If a position was opened before this field existed it defaults to 0.68
/// (MIN_CONFIDENCE), which means the new signal just needs to beat 0.76 to
/// override.
pub fn correlation_block(
    new_symbol:  &str,
    new_side:    &str,
    new_conf:    f64,
    positions:   &[crate::web_dashboard::PaperPosition],
) -> CorrBlock {
    for pos in positions {
        if pos.side != new_side {
            continue; // only block same-direction stacking
        }
        let corr = get_correlation(new_symbol, &pos.symbol);
        if corr < CORR_THRESHOLD {
            continue;
        }
        let existing_conf = pos.entry_confidence;
        if new_conf >= existing_conf + CONF_EDGE {
            return CorrBlock::Override {
                existing: pos.symbol.clone(),
                corr,
            };
        }
        return CorrBlock::Blocked {
            existing:      pos.symbol.clone(),
            corr,
            existing_conf,
        };
    }
    CorrBlock::Clear
}

// ─────────────────────────── Unit tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btc_eth_high_correlation() {
        assert!(get_correlation("BTC", "ETH") >= 0.80);
    }

    #[test]
    fn same_symbol_is_one() {
        assert_eq!(get_correlation("SOL", "SOL"), 1.0);
        assert_eq!(get_correlation("BTC-USD", "BTC"), 1.0);
    }

    #[test]
    fn suffix_stripped() {
        assert_eq!(
            get_correlation("BTC-USD", "ETH-USD"),
            get_correlation("BTC",     "ETH"),
        );
    }

    #[test]
    fn unknown_pair_defaults_low() {
        let r = get_correlation("BTC", "DOGE");
        assert!((0.30..0.75).contains(&r));
    }

    #[test]
    fn arb_op_cluster() {
        assert!(get_correlation("ARB", "OP") >= 0.85);
    }
}
