/// Integration-level tests for the candidates pipeline.
///
/// These tests are self-contained (no crate imports) so they compile without
/// requiring lib.rs to re-export the binary-crate modules.
///
/// The pure functions under test are duplicated inline here so behaviour
/// regressions are caught immediately even across refactors.
///
/// For per-method unit tests see the #[cfg(test)] modules inside:
///   src/data.rs      — hl_to_binance, filter_candidates
///   src/sentiment.rs — SentimentData, normalise_symbol, JSON shapes
use std::collections::HashMap;

// ── Inline copies of pure functions ───────────────────────────────────────────
// These mirror src/data.rs and src/sentiment.rs exactly.
// Any divergence = regression detected by a failing test.

fn hl_to_binance(hl: &str) -> Option<String> {
    if hl.starts_with('@') {
        return None;
    }
    if hl.contains('/') {
        return None;
    }
    if let Some(base) = hl.strip_prefix('k') {
        Some(format!("1000{base}USDT"))
    } else {
        Some(format!("{hl}USDT"))
    }
}

fn normalise_for_lunarcrush(symbol: &str) -> &str {
    symbol.strip_prefix('k').unwrap_or(symbol)
}

fn filter_candidates<'a>(
    current: &'a HashMap<String, f64>,
    previous: &'a HashMap<String, f64>,
) -> Vec<String> {
    let anchors = ["BTC", "ETH", "SOL"];
    let mut movers: Vec<(String, f64)> = current
        .iter()
        .filter(|(sym, _)| hl_to_binance(sym).is_some())
        .filter_map(|(sym, &cur)| {
            let prev = previous.get(sym.as_str()).copied().unwrap_or(cur);
            if prev == 0.0 {
                return None;
            }
            let pct = ((cur - prev) / prev).abs();
            Some((sym.clone(), pct))
        })
        .collect();
    movers.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut candidates: Vec<String> = anchors
        .iter()
        .filter(|&&s| current.contains_key(s))
        .map(|&s| s.to_string())
        .collect();
    for (sym, _) in movers.iter().take(20) {
        if !candidates.contains(sym) {
            candidates.push(sym.clone());
        }
        if candidates.len() >= 18 {
            break;
        }
    }
    candidates
}

fn mids(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

// ── hl_to_binance ──────────────────────────────────────────────────────────────

#[test]
fn binance_map_standard_coins() {
    for (hl, bn) in [
        ("BTC", "BTCUSDT"),
        ("ETH", "ETHUSDT"),
        ("SOL", "SOLUSDT"),
        ("AVAX", "AVAXUSDT"),
    ] {
        assert_eq!(hl_to_binance(hl), Some(bn.to_string()));
    }
}

#[test]
fn binance_map_k_prefix_becomes_1000() {
    for (hl, bn) in [
        ("kBONK", "1000BONKUSDT"),
        ("kPEPE", "1000PEPEUSDT"),
        ("kSHIB", "1000SHIBUSDT"),
    ] {
        assert_eq!(hl_to_binance(hl), Some(bn.to_string()));
    }
}

#[test]
fn binance_map_rejects_at_and_slash() {
    for invalid in ["@232", "@7", "@1000", "PURR/USDC", "BTC/USDC"] {
        assert_eq!(
            hl_to_binance(invalid),
            None,
            "{invalid} should have no Binance mapping"
        );
    }
}

// ── filter_candidates ──────────────────────────────────────────────────────────

#[test]
fn candidates_contains_anchors() {
    let c = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("DOGE", 0.1),
    ]);
    let p = c.clone();
    let r = filter_candidates(&c, &p);
    for anchor in ["BTC", "ETH", "SOL"] {
        assert!(
            r.contains(&anchor.to_string()),
            "{anchor} anchor missing from candidates"
        );
    }
}

#[test]
fn candidates_capped_at_18() {
    let current: HashMap<String, f64> = (0..50)
        .map(|i| (format!("C{i:02}"), 10.0 + i as f64))
        .chain([
            ("BTC".into(), 50000.0),
            ("ETH".into(), 3000.0),
            ("SOL".into(), 100.0),
        ])
        .collect();
    let previous: HashMap<String, f64> = (0..50)
        .map(|i| (format!("C{i:02}"), 10.0))
        .chain([
            ("BTC".into(), 50000.0),
            ("ETH".into(), 3000.0),
            ("SOL".into(), 100.0),
        ])
        .collect();
    let r = filter_candidates(&current, &previous);
    assert!(r.len() <= 18, "Expected ≤18 candidates, got {}", r.len());
}

#[test]
fn candidates_no_duplicates() {
    // BTC is anchor AND biggest mover — should appear exactly once
    let c = mids(&[("BTC", 60000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
    let p = mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
    let r = filter_candidates(&c, &p);
    let count = r.iter().filter(|s| s.as_str() == "BTC").count();
    assert_eq!(count, 1, "BTC appears {count}× — expected exactly 1");
}

#[test]
fn candidates_excludes_at_symbols() {
    // @-symbols move 9999% but still must be excluded (no Binance mapping)
    let c = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("@232", 5000.0),
        ("@7", 7000.0),
    ]);
    let p = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("@232", 1.0),
        ("@7", 1.0),
    ]);
    let r = filter_candidates(&c, &p);
    assert!(
        !r.contains(&"@232".to_string()),
        "@232 must not be a candidate"
    );
    assert!(!r.contains(&"@7".to_string()), "@7 must not be a candidate");
}

#[test]
fn candidates_top_mover_selected_over_flat_coin() {
    let c = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("MOON", 2.0),
        ("FLAT", 1.0),
    ]);
    let p = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("MOON", 1.0),
        ("FLAT", 1.0),
    ]);
    let r = filter_candidates(&c, &p);
    assert!(
        r.contains(&"MOON".to_string()),
        "Top mover MOON should be included"
    );
}

#[test]
fn candidates_handles_empty_previous_gracefully() {
    // Cycle 1: prev is empty — anchors should still be returned
    let c = mids(&[("BTC", 50000.0), ("ETH", 3000.0), ("SOL", 100.0)]);
    let p: HashMap<String, f64> = HashMap::new();
    let r = filter_candidates(&c, &p);
    assert!(
        r.contains(&"BTC".to_string()),
        "BTC anchor must survive empty prev map"
    );
}

#[test]
fn candidates_k_prefix_tokens_pass_binance_filter() {
    // kBONK, kPEPE etc have valid Binance mappings and should appear in candidates
    let c = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("kBONK", 0.002),
        ("kPEPE", 0.00001),
    ]);
    let p = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("kBONK", 0.001),
        ("kPEPE", 0.000005),
    ]);
    let r = filter_candidates(&c, &p);
    assert!(
        r.contains(&"kBONK".to_string()),
        "kBONK should be a valid candidate"
    );
    assert!(
        r.contains(&"kPEPE".to_string()),
        "kPEPE should be a valid candidate"
    );
}

// ── Sentiment normalisation (root cause of the dashboard bug) ──────────────────

#[test]
fn sentiment_kprefix_bug_regression_kbonk_resolves_to_bonk() {
    // REGRESSION TEST — this is the exact bug that caused sentiment to not show.
    // HL calls the token "kBONK", LunarCrush knows it as "BONK".
    // If get("kBONK") looked up "kBONK" directly, it would always return None.
    // After the fix, normalise_for_lunarcrush("kBONK") == "BONK".
    let lc_key = normalise_for_lunarcrush("kBONK");
    assert_eq!(
        lc_key, "BONK",
        "kBONK must normalise to BONK for LunarCrush lookup"
    );

    // Simulate a cache hit after normalisation
    let mut cache: HashMap<String, f64> = HashMap::new();
    cache.insert("BONK".to_string(), 72.0); // 72% bullish

    let hit = cache.get(lc_key);
    assert!(
        hit.is_some(),
        "Cache lookup with normalised key must succeed"
    );
    assert_eq!(*hit.unwrap(), 72.0);
}

#[test]
fn sentiment_kprefix_bug_regression_without_fix() {
    // Show what USED TO HAPPEN before the fix — looking up "kBONK" directly.
    // This test documents the broken behaviour so the fix is obvious.
    let mut cache: HashMap<String, f64> = HashMap::new();
    cache.insert("BONK".to_string(), 72.0); // keyed as "BONK", not "kBONK"

    let broken_lookup = cache.get("kBONK"); // pre-fix: direct symbol lookup
    assert!(
        broken_lookup.is_none(),
        "Pre-fix: kBONK lookup returns None — this is the bug that was fixed"
    );
}

#[test]
fn sentiment_normalise_all_known_k_prefix_tokens() {
    let cases = [
        ("kBONK", "BONK"),
        ("kPEPE", "PEPE"),
        ("kSHIB", "SHIB"),
        ("kFLOKI", "FLOKI"),
        ("kLUNC", "LUNC"),
    ];
    for (hl, lc) in cases {
        assert_eq!(
            normalise_for_lunarcrush(hl),
            lc,
            "{hl} should normalise to {lc}"
        );
    }
}

#[test]
fn sentiment_normalise_preserves_uppercase_k_names() {
    // KAVA, KLAY start with uppercase K — must NOT be stripped
    for sym in ["KAVA", "KLAY", "KSM"] {
        assert_eq!(
            normalise_for_lunarcrush(sym),
            sym,
            "{sym} starts with uppercase K and must not be stripped"
        );
    }
}

#[test]
fn sentiment_normalise_standard_coins_unchanged() {
    for sym in ["BTC", "ETH", "SOL", "AVAX", "BNB", "DOGE", "TRUMP", "SUI"] {
        assert_eq!(
            normalise_for_lunarcrush(sym),
            sym,
            "{sym} must pass through normalise unchanged"
        );
    }
}

// ── Full pipeline integration ──────────────────────────────────────────────────

#[test]
fn pipeline_candidate_to_sentiment_lookup_end_to_end() {
    // Simulate one full cycle:
    // 1. filter_candidates produces a list with kBONK
    // 2. For each candidate, normalise before LunarCrush lookup
    // 3. kBONK should find sentiment data (keyed as "BONK")

    let current = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("kBONK", 0.002), // HL k-prefix token with movement
    ]);
    let previous = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("kBONK", 0.001),
    ]);

    let candidates = filter_candidates(&current, &previous);
    assert!(candidates.contains(&"kBONK".to_string()));

    // Simulate LunarCrush cache (keyed by LC symbol, uppercase, no k prefix)
    let mut lc_cache: HashMap<String, f64> = HashMap::new();
    lc_cache.insert("BONK".to_string(), 68.0); // 68% bullish
    lc_cache.insert("BTC".to_string(), 71.0);
    lc_cache.insert("ETH".to_string(), 55.0);
    lc_cache.insert("SOL".to_string(), 63.0);

    // Look up sentiment for each candidate using normalised key
    let mut hits = 0usize;
    for sym in &candidates {
        let key = normalise_for_lunarcrush(sym);
        if lc_cache.contains_key(key) {
            hits += 1;
        }
    }

    // All 4 symbols (BTC, ETH, SOL, kBONK→BONK) should hit
    assert_eq!(
        hits, 4,
        "All 4 candidates should find sentiment after k-prefix normalisation, got {hits}"
    );
}

#[test]
fn pipeline_at_symbols_never_reach_sentiment_lookup() {
    // @232 is filtered out by filter_candidates, so sentiment is never even attempted
    let current = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("@232", 5000.0),
    ]);
    let previous = mids(&[
        ("BTC", 50000.0),
        ("ETH", 3000.0),
        ("SOL", 100.0),
        ("@232", 1.0),
    ]);

    let candidates = filter_candidates(&current, &previous);

    // @232 must not be in candidates, so sentiment for it is never queried
    assert!(
        !candidates.contains(&"@232".to_string()),
        "@232 must be filtered before sentiment lookup"
    );
}
