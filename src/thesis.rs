//! Investment Thesis — user-controlled trading constraints.
//!
//! Users enter natural-language commands via the floating AI bar in the
//! consumer app.  This module parses those commands into a `ThesisConstraints`
//! struct that the trading loop reads each cycle to:
//!
//!   1. Filter the candidate list to whitelisted symbols / sectors.
//!   2. Clamp leverage after `analyse_symbol` returns.
//!
//! Parsing is entirely keyword-based (no LLM call required) — instant, free,
//! and deterministic.  The user's raw command text is stored for display.
//!
//! # Thread-safety
//!
//! The canonical live copy is `Arc<RwLock<ThesisConstraints>>`, shared
//! between the Axum web handlers (write) and `run_cycle` (read).

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────

/// The parsed set of user-imposed trading constraints.
///
/// All fields are optional — `None` means "use the AI default".
/// `Default::default()` (all `None`) means no constraints are active.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThesisConstraints {
    /// Short human-readable summary shown as the active chip in the UI.
    /// e.g. `"Only BTC · ETH · SOL"` or `"Meme coins · max 3×"`
    pub summary: Option<String>,

    /// Explicit symbol whitelist.  When set, `run_cycle` skips any candidate
    /// whose symbol is NOT in this list.
    pub symbol_whitelist: Option<Vec<String>>,

    /// Named sector filter.  Applied when `symbol_whitelist` is `None`.
    /// Supported values: `"meme"`, `"l1"`, `"l2"`, `"defi"`, `"rwa"`.
    pub sector_filter: Option<String>,

    /// Maximum leverage the bot is allowed to use.
    /// `dec.leverage = dec.leverage.min(max_leverage_override)` after analysis.
    pub max_leverage_override: Option<f64>,

    /// Original free-text command from the user (stored for display).
    pub thesis_text: Option<String>,
}

impl ThesisConstraints {
    /// Returns `true` if no constraints are active (pure AI mode).
    pub fn is_empty(&self) -> bool {
        self.symbol_whitelist.is_none()
            && self.sector_filter.is_none()
            && self.max_leverage_override.is_none()
    }

    /// Given a candidate symbol, return `true` if it should be traded.
    ///
    /// Returns `true` when:
    ///   - No whitelist or sector filter is active, OR
    ///   - `symbol_whitelist` is set and contains `sym`, OR
    ///   - `sector_filter` is set and `sector_symbols(sector)` contains `sym`.
    pub fn allows(&self, sym: &str) -> bool {
        if let Some(ref list) = self.symbol_whitelist {
            return list.iter().any(|s| s.eq_ignore_ascii_case(sym));
        }
        if let Some(ref sector) = self.sector_filter {
            if let Some(syms) = sector_symbols(sector) {
                return syms.iter().any(|s| s.eq_ignore_ascii_case(sym));
            }
        }
        true // no filter active
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Sector → symbol mapping
// ─────────────────────────────────────────────────────────────────────────────

/// Return the symbol list for a named sector, or `None` if the sector is
/// unknown.  All symbols are uppercase and match Hyperliquid perp names.
pub fn sector_symbols(sector: &str) -> Option<Vec<String>> {
    let syms: &[&str] = match sector.to_ascii_lowercase().as_str() {
        "meme" | "memes" | "memecoin" | "memecoins" => &[
            "PEPE", "WIF", "BONK", "DOGE", "SHIB", "FLOKI", "BRETT", "MOODENG", "NEIRO", "GOAT",
            "MEME", "PONKE", "BOME", "PNUT",
        ],
        "l1" | "layer1" | "layer-1" => &[
            "BTC", "ETH", "SOL", "BNB", "AVAX", "ADA", "DOT", "ATOM", "NEAR", "SUI", "APT", "SEI",
        ],
        "l2" | "layer2" | "layer-2" => &[
            "ARB", "OP", "MATIC", "STRK", "BLAST", "ZK", "SCROLL", "BASE",
        ],
        "defi" => &[
            "UNI", "AAVE", "CRV", "MKR", "SNX", "COMP", "SUSHI", "BAL", "JUP", "ORCA", "DRIFT",
            "GMX", "DYDX",
        ],
        "rwa" | "real-world" | "realworld" => &["ONDO", "MPL", "CPOOL", "RIO", "POLYX"],
        "ai" | "aicoins" | "ai-coins" => {
            &["FET", "AGIX", "OCEAN", "RNDR", "WLD", "TAO", "AIOZ", "GRT"]
        }
        _ => return None,
    };
    Some(syms.iter().map(|s| s.to_string()).collect())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Command parser
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a natural-language command string into `ThesisConstraints`.
///
/// Rules (applied in order, left-to-right, all case-insensitive):
///
/// 1. **Reset** — "reset", "clear", "no constraint", "remove" → all fields cleared.
/// 2. **Leverage** — "Nx", "N× leverage", "max N leverage", "reduce risk",
///    "more risk", "safe", "conservative" → sets `max_leverage_override`.
/// 3. **Sector** — "meme coins", "memes", "layer 1", "defi", etc.
/// 4. **Explicit symbols** — "only BTC ETH SOL", "trade BTC,ETH", comma/space lists.
/// 5. **Query intent** — "what did you trade", "show trades" → returns `None`
///    (caller should redirect to trade-query path, not update constraints).
///
/// Returns `None` if the command appears to be a trade query rather than a
/// constraint update (so the caller can handle it differently).
pub fn parse_command(cmd: &str) -> Option<ThesisConstraints> {
    // Hard length cap — the web handler already checks this, but belt-and-braces.
    if cmd.len() > 200 {
        return Some(ThesisConstraints::default());
    }

    let lc = cmd.to_ascii_lowercase();

    // ── Trade queries (do NOT update constraints) ─────────────────────────
    let query_phrases = [
        "what did you trade",
        "show trades",
        "show my trades",
        "recent trades",
        "last trade",
        "what trades",
        "trade history",
        "what have you",
        "what have i",
        "show positions",
        "my positions",
    ];
    if query_phrases.iter().any(|p| lc.contains(p)) {
        return None; // signal: treat as a query, not a constraint update
    }

    // ── Reset ────────────────────────────────────────────────────────────
    let reset_phrases = [
        "reset",
        "clear",
        "remove",
        "no constraint",
        "default",
        "ai default",
        "let ai decide",
        "ai decides",
        "auto",
    ];
    if reset_phrases.iter().any(|p| lc.contains(p)) {
        return Some(ThesisConstraints::default());
    }

    let mut out = ThesisConstraints {
        thesis_text: Some(cmd.to_string()),
        ..Default::default()
    };

    // ── Leverage extraction ───────────────────────────────────────────────
    // Patterns: "5x", "5×", "5 x leverage", "max 5x", "max leverage 5"
    let lev = extract_leverage(&lc);
    // Named risk adjectives
    let lev = lev.or_else(|| {
        if lc.contains("conservative") || lc.contains("safe") || lc.contains("low risk") {
            Some(2.0)
        } else if lc.contains("reduce risk") || lc.contains("less risk") {
            Some(3.0)
        } else if lc.contains("high risk") || lc.contains("more risk") || lc.contains("aggressive")
        {
            Some(10.0)
        } else {
            None
        }
    });
    out.max_leverage_override = lev;

    // ── Sector detection ─────────────────────────────────────────────────
    let sector_keywords: &[(&str, &str)] = &[
        ("meme coin", "meme"),
        ("meme coins", "meme"),
        ("memecoin", "meme"),
        ("memes", "meme"),
        ("layer 1", "l1"),
        ("layer1", "l1"),
        ("layer-1", "l1"),
        ("l1 coin", "l1"),
        ("layer 2", "l2"),
        ("layer2", "l2"),
        ("layer-2", "l2"),
        ("defi", "defi"),
        ("decentralised finance", "defi"),
        ("decentralized finance", "defi"),
        ("real world asset", "rwa"),
        ("real-world asset", "rwa"),
        ("rwa", "rwa"),
        ("ai coin", "ai"),
        ("ai coins", "ai"),
        ("artificial intelligence", "ai"),
    ];
    for (kw, sector) in sector_keywords {
        if lc.contains(kw) {
            out.sector_filter = Some(sector.to_string());
            break;
        }
    }

    // ── Explicit symbol extraction ────────────────────────────────────────
    // Only parse explicit symbols if no sector was detected (avoid conflicts).
    // Recognises: "only BTC ETH SOL", "only BTC/ETH/SOL", "only BTC,ETH,SOL"
    // Symbol candidates are 2–6 uppercase ASCII letter sequences from the
    // original (non-lowercased) command.
    if out.sector_filter.is_none() {
        let syms = extract_symbols(cmd);
        if !syms.is_empty() {
            out.symbol_whitelist = Some(syms);
        }
    }

    // ── Summary line ─────────────────────────────────────────────────────
    out.summary = build_summary(&out);

    Some(out)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Extract a leverage multiplier from the command string.
/// Handles: "5x", "5×", "5X", "5 x", "max 5", "leverage 5", "5-x"
fn extract_leverage(lc: &str) -> Option<f64> {
    // Walk the string looking for a digit sequence followed by optional
    // whitespace / dash / dot and then 'x' or '×'.
    let bytes = lc.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            // Collect the number (may include one decimal point)
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let num_str = &lc[start..i];
            // Skip optional whitespace / dash
            let mut j = i;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'-') {
                j += 1;
            }
            // Check for 'x' or '×' (multi-byte UTF-8: 0xC3 0x97)
            let is_x = j < bytes.len() && (bytes[j] == b'x' || bytes[j] == b'X');
            let is_times = j + 1 < bytes.len() && bytes[j] == 0xC3 && bytes[j + 1] == 0x97;
            if is_x || is_times {
                if let Ok(v) = num_str.parse::<f64>() {
                    if (1.0..=50.0).contains(&v) {
                        return Some(v);
                    }
                }
            }
        }
        i += 1;
    }

    // Also handle "max N leverage" / "leverage N" with no 'x' suffix
    for pattern in &["max leverage ", "max lev ", "leverage ", "lev "] {
        if let Some(pos) = lc.find(pattern) {
            let rest = &lc[pos + pattern.len()..];
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = num.parse::<f64>() {
                if (1.0..=50.0).contains(&v) {
                    return Some(v);
                }
            }
        }
    }

    None
}

/// Extract recognised crypto ticker symbols from the raw (mixed-case) command.
///
/// A "symbol candidate" is a run of 2–6 ASCII uppercase letters that:
///   - Appears in the Hyperliquid perp universe (known-symbols filter), OR
///   - Is preceded by a trigger word like "only", "trade", "just".
///
/// We use a simple known-symbol allowlist so that common English words like
/// "ETH" (ambiguous) are accepted while "BUY", "MAX", "RISK" are rejected.
fn extract_symbols(cmd: &str) -> Vec<String> {
    /// All Hyperliquid perp symbols we're willing to accept as a whitelist.
    const KNOWN: &[&str] = &[
        "BTC", "ETH", "SOL", "BNB", "AVAX", "ADA", "DOT", "ATOM", "NEAR", "SUI", "APT", "SEI",
        "PEPE", "WIF", "BONK", "DOGE", "SHIB", "FLOKI", "BRETT", "MOODENG", "NEIRO", "GOAT",
        "MEME", "PONKE", "BOME", "PNUT", "ARB", "OP", "MATIC", "STRK", "BLAST", "ZK", "UNI",
        "AAVE", "CRV", "MKR", "SNX", "COMP", "SUSHI", "JUP", "ORCA", "DRIFT", "GMX", "DYDX",
        "ONDO", "FET", "AGIX", "OCEAN", "RNDR", "WLD", "TAO", "GRT", "LTC", "XRP", "LINK", "FTM",
        "INJ", "TIA", "PYTH", "JTO", "MANTA", "ALT", "DYM", "PIXEL", "PORTAL", "WBTC", "ORDI",
        "SATS", "ENA", "ETHFI", "EIGEN", "CELO", "IMX", "ICP", "LDO", "RPL", "PENDLE", "BANANA",
        "TURBO", "MOG", "POPCAT", "FWOG", "GIGA", "MICHI",
    ];

    let mut found = Vec::new();
    // Split on common delimiters (space, comma, slash, +, |)
    let parts: Vec<&str> = cmd.split(|c: char| !c.is_alphanumeric()).collect();
    for part in parts {
        let up = part.to_ascii_uppercase();
        if up.len() >= 2 && up.len() <= 6 && KNOWN.contains(&up.as_str()) && !found.contains(&up) {
            found.push(up);
        }
    }
    found
}

/// Build a compact display summary from the parsed constraints.
fn build_summary(c: &ThesisConstraints) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(ref list) = c.symbol_whitelist {
        parts.push(format!("Only: {}", list.join(" · ")));
    } else if let Some(ref sector) = c.sector_filter {
        let label = match sector.as_str() {
            "meme" => "Meme coins",
            "l1" => "Layer-1",
            "l2" => "Layer-2",
            "defi" => "DeFi",
            "rwa" => "RWA",
            "ai" => "AI coins",
            other => other,
        };
        parts.push(label.to_string());
    }

    if let Some(lev) = c.max_leverage_override {
        if (lev - lev.floor()).abs() < 0.01 {
            parts.push(format!("max {}×", lev as u32));
        } else {
            parts.push(format!("max {:.1}×", lev));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Reset / clear ─────────────────────────────────────────────────────

    #[test]
    fn reset_clears_all_constraints() {
        let c = parse_command("reset").unwrap();
        assert!(c.is_empty(), "reset must produce empty constraints");
        assert!(c.summary.is_none());
    }

    #[test]
    fn clear_keyword_resets() {
        let c = parse_command("clear everything").unwrap();
        assert!(c.is_empty());
    }

    #[test]
    fn default_constraints_allows_any_symbol() {
        let c = ThesisConstraints::default();
        assert!(c.allows("BTC"));
        assert!(c.allows("PEPE"));
        assert!(c.allows("UNKNOWN_COIN"));
    }

    // ── Leverage extraction ───────────────────────────────────────────────

    #[test]
    fn parses_5x_leverage() {
        let c = parse_command("max 5x leverage").unwrap();
        assert_eq!(c.max_leverage_override, Some(5.0));
    }

    #[test]
    fn parses_3x_with_unicode_times() {
        let c = parse_command("trade with 3× leverage").unwrap();
        assert_eq!(c.max_leverage_override, Some(3.0));
    }

    #[test]
    fn parses_decimal_leverage() {
        let c = parse_command("2.5x risk").unwrap();
        assert_eq!(c.max_leverage_override, Some(2.5));
    }

    #[test]
    fn conservative_maps_to_2x() {
        let c = parse_command("be conservative").unwrap();
        assert_eq!(c.max_leverage_override, Some(2.0));
    }

    #[test]
    fn more_risk_maps_to_10x() {
        let c = parse_command("I want more risk").unwrap();
        assert_eq!(c.max_leverage_override, Some(10.0));
    }

    #[test]
    fn no_leverage_keyword_leaves_none() {
        let c = parse_command("only trade BTC ETH").unwrap();
        assert_eq!(c.max_leverage_override, None);
    }

    // ── Sector detection ─────────────────────────────────────────────────

    #[test]
    fn meme_coins_phrase_sets_sector() {
        let c = parse_command("only invest in meme coins").unwrap();
        assert_eq!(c.sector_filter.as_deref(), Some("meme"));
        assert!(
            c.symbol_whitelist.is_none(),
            "sector overrides symbol extraction"
        );
    }

    #[test]
    fn memecoin_without_space_sets_sector() {
        let c = parse_command("trade memecoins only").unwrap();
        assert_eq!(c.sector_filter.as_deref(), Some("meme"));
    }

    #[test]
    fn layer1_sets_l1_sector() {
        let c = parse_command("focus on layer 1 coins").unwrap();
        assert_eq!(c.sector_filter.as_deref(), Some("l1"));
    }

    #[test]
    fn defi_sector_detected() {
        let c = parse_command("trade DeFi protocols only").unwrap();
        assert_eq!(c.sector_filter.as_deref(), Some("defi"));
    }

    // ── Sector symbols ────────────────────────────────────────────────────

    #[test]
    fn meme_sector_contains_pepe_and_wif() {
        let syms = sector_symbols("meme").unwrap();
        assert!(syms.contains(&"PEPE".to_string()));
        assert!(syms.contains(&"WIF".to_string()));
    }

    #[test]
    fn l1_sector_contains_btc_eth_sol() {
        let syms = sector_symbols("l1").unwrap();
        assert!(syms.contains(&"BTC".to_string()));
        assert!(syms.contains(&"ETH".to_string()));
        assert!(syms.contains(&"SOL".to_string()));
    }

    #[test]
    fn unknown_sector_returns_none() {
        assert!(sector_symbols("sportscoins").is_none());
    }

    // ── allows() filter ───────────────────────────────────────────────────

    #[test]
    fn whitelist_allows_only_listed_symbols() {
        let c = ThesisConstraints {
            symbol_whitelist: Some(vec!["BTC".into(), "ETH".into()]),
            ..Default::default()
        };
        assert!(c.allows("BTC"));
        assert!(c.allows("ETH"));
        assert!(!c.allows("SOL"));
        assert!(!c.allows("PEPE"));
    }

    #[test]
    fn sector_filter_allows_only_sector_symbols() {
        let c = ThesisConstraints {
            sector_filter: Some("meme".into()),
            ..Default::default()
        };
        assert!(c.allows("PEPE"));
        assert!(c.allows("WIF"));
        assert!(!c.allows("BTC"));
        assert!(!c.allows("UNI"));
    }

    // ── Explicit symbol extraction ────────────────────────────────────────

    #[test]
    fn only_btc_eth_sol_parsed() {
        let c = parse_command("only trade BTC ETH SOL").unwrap();
        let list = c.symbol_whitelist.unwrap();
        assert!(list.contains(&"BTC".to_string()));
        assert!(list.contains(&"ETH".to_string()));
        assert!(list.contains(&"SOL".to_string()));
    }

    #[test]
    fn comma_separated_symbols_parsed() {
        let c = parse_command("only BTC,ETH,SOL").unwrap();
        let list = c.symbol_whitelist.unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn unknown_tickers_not_included() {
        // "ROCK" and "MOON" are not in the known-symbols list
        let c = parse_command("only trade ROCK and MOON").unwrap();
        assert!(c.symbol_whitelist.is_none() || c.symbol_whitelist.as_ref().unwrap().is_empty());
    }

    // ── Combined constraints ──────────────────────────────────────────────

    #[test]
    fn sector_plus_leverage_combined() {
        let c = parse_command("only meme coins with max 3x leverage").unwrap();
        assert_eq!(c.sector_filter.as_deref(), Some("meme"));
        assert_eq!(c.max_leverage_override, Some(3.0));
        let summary = c.summary.unwrap();
        assert!(summary.contains("Meme coins"));
        assert!(summary.contains("max 3×"));
    }

    #[test]
    fn btc_eth_only_generates_summary() {
        let c = parse_command("only BTC and ETH").unwrap();
        let summary = c.summary.unwrap();
        assert!(summary.contains("BTC"));
        assert!(summary.contains("ETH"));
    }

    // ── Trade query detection ─────────────────────────────────────────────

    #[test]
    fn trade_query_returns_none() {
        assert!(parse_command("what did you trade today?").is_none());
        assert!(parse_command("show my trades").is_none());
        assert!(parse_command("recent trades").is_none());
    }

    // ── Summary formatting ────────────────────────────────────────────────

    #[test]
    fn integer_leverage_shows_no_decimal() {
        let c = parse_command("max 5x").unwrap();
        assert!(c.summary.unwrap().contains("max 5×"));
    }

    #[test]
    fn decimal_leverage_shows_one_decimal() {
        let c = parse_command("2.5x risk").unwrap();
        assert!(c.summary.unwrap().contains("max 2.5×"));
    }
}
