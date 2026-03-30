//! Append-only CSV trade ledger.
//!
//! Every closed trade (full close **or** partial tranche) is written as a
//! single line to `trades_YYYY.csv` in the working directory.  A fresh file
//! is started each calendar year so exports stay manageable.
//!
//! The ledger is the source of truth for tax reporting — it is never truncated
//! and survives BotState resets.
//!
//! ## CSV schema (per row)
//!
//! | # | Field          | Example              | Notes                          |
//! |---|----------------|----------------------|--------------------------------|
//! | 0 | date           | 2025-11-15           | UTC close date                 |
//! | 1 | time_utc       | 14:32:07             | UTC close time                 |
//! | 2 | symbol         | SOL                  | Base asset                     |
//! | 3 | side           | LONG                 | LONG or SHORT                  |
//! | 4 | entry_price    | 183.4200             | USD per unit at open           |
//! | 5 | exit_price     | 201.8900             | USD per unit at close          |
//! | 6 | quantity       | 0.548000             | Units of base asset            |
//! | 7 | size_usd       | 100.52               | Margin committed (USD)         |
//! | 8 | leverage       | 2.00                 | e.g. 2.0×                      |
//! | 9 | notional_usd   | 201.04               | size_usd × leverage            |
//! |10 | gross_pnl      | +10.14               | Before fees                    |
//! |11 | fees_est       | -0.15                | ~0.075 % of notional round-trip|
//! |12 | net_pnl        | +9.99                | gross_pnl – fees_est           |
//! |13 | pnl_pct        | +9.94                | net_pnl / size_usd × 100       |
//! |14 | reason         | TakeProfit2R         | Close trigger                  |
//! |15 | entry_time     | 2025-11-15T13:01:22Z | ISO-8601 open time             |
//!
//! ## Fee estimate methodology
//!
//! Hyperliquid charges approximately:
//!   - 0.020 % maker / 0.050 % taker on notional per leg
//!   - Builder fee: 0.01 % (1 bp) on notional per leg
//!
//! We conservatively estimate **0.075 % × notional × 2 legs** = 0.15 % of
//! notional per round-trip.  Actual fees will appear on your HL statement.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::web_dashboard::ClosedTrade;

/// Fee rate applied to each leg: maker(0.02%) + builder(0.01%) + buffer.
const FEE_RATE_PER_LEG: f64 = 0.00075; // 0.075 % × 2 legs = 0.15 % round-trip

// ─────────────────────────────────────────────────────────────────────────────

/// Estimate round-trip fees for a trade.
///
/// `notional` = size_usd × leverage
pub fn estimate_fees(size_usd: f64, leverage: f64) -> f64 {
    let notional = size_usd * leverage;
    notional * FEE_RATE_PER_LEG * 2.0
}

// ─────────────────────────────────────────────────────────────────────────────

/// Path to the current-year CSV file.
fn ledger_path(year: i32) -> PathBuf {
    Path::new(&format!("trades_{}.csv", year)).to_path_buf()
}

/// CSV header row.
const HEADER: &str = "date,time_utc,symbol,side,entry_price,exit_price,quantity,size_usd,leverage,\
     notional_usd,gross_pnl,fees_est,net_pnl,pnl_pct,reason,entry_time\n";

/// Append a closed trade to the annual CSV ledger.
///
/// Creates the file (with header) on first write.  Thread-safe via filesystem
/// atomicity — each call opens, appends one line, and closes immediately.
pub fn append(trade: &ClosedTrade) {
    // Determine year from closed_at (ISO-8601 prefix)
    let year: i32 = trade
        .closed_at
        .split('-')
        .next()
        .and_then(|y| y.parse().ok())
        .unwrap_or_else(|| {
            chrono::Utc::now()
                .format("%Y")
                .to_string()
                .parse()
                .unwrap_or(2025)
        });

    let path = ledger_path(year);
    let is_new = !path.exists();

    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Ledger open failed: {}", e);
            return;
        }
    };

    if is_new {
        if let Err(e) = file.write_all(HEADER.as_bytes()) {
            log::error!("Ledger header write failed: {}", e);
            return;
        }
    }

    let notional = trade.size_usd * trade.leverage.max(1.0);
    let fees = if trade.fees_est > 0.0 {
        trade.fees_est
    } else {
        estimate_fees(trade.size_usd, trade.leverage.max(1.0))
    };
    let net_pnl = trade.pnl - fees;
    let pnl_pct = if trade.size_usd > 1e-8 {
        net_pnl / trade.size_usd * 100.0
    } else {
        trade.pnl_pct
    };

    // date and time from closed_at (strip time-zone suffix for readability)
    let closed_clean = trade.closed_at.replace('T', " ").replace('Z', "");
    let parts: Vec<&str> = closed_clean.splitn(2, ' ').collect();
    let date = parts.first().copied().unwrap_or(&trade.closed_at);
    let time = parts.get(1).copied().unwrap_or("00:00:00");

    let entry_time_clean = trade.entry_time.replace('T', " ").replace('Z', "");
    let entry_iso = if entry_time_clean.is_empty() {
        "unknown".to_string()
    } else {
        trade.entry_time.clone()
    };

    let row = format!(
        "{date},{time},{sym},{side},{entry:.4},{exit:.4},{qty:.6},{size:.2},{lev:.2},{notional:.2},{gross:+.4},{fees:.4},{net:+.4},{pct:+.2},{reason},{entry_time}\n",
        date        = date,
        time        = time,
        sym         = trade.symbol,
        side        = trade.side,
        entry       = trade.entry,
        exit        = trade.exit,
        qty         = trade.quantity,
        size        = trade.size_usd,
        lev         = trade.leverage.max(1.0),
        notional    = notional,
        gross       = trade.pnl,
        fees        = fees,
        net         = net_pnl,
        pct         = pnl_pct,
        reason      = trade.reason,
        entry_time  = entry_iso,
    );

    if let Err(e) = file.write_all(row.as_bytes()) {
        log::error!("Ledger row write failed: {}", e);
    } else {
        log::debug!(
            "📒 Ledger: {} {} {}",
            trade.symbol,
            trade.side,
            trade.closed_at
        );
    }
}

/// Read all rows from all annual ledger files as raw CSV text.
/// Returns `(csv_text, row_count)`.  Used by the tax report endpoint.
pub fn read_all() -> (String, usize) {
    let mut output = String::from(HEADER);
    let mut row_count = 0usize;

    // Scan for trades_YYYY.csv files in the working directory
    let current_year = chrono::Utc::now()
        .format("%Y")
        .to_string()
        .parse::<i32>()
        .unwrap_or(2025);
    for year in 2024..=(current_year + 1) {
        let path = ledger_path(year);
        if !path.exists() {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                // Skip header line for all but the first file
                for line in content.lines().skip(1) {
                    if !line.is_empty() {
                        output.push_str(line);
                        output.push('\n');
                        row_count += 1;
                    }
                }
            }
            Err(e) => log::warn!("Ledger read {} failed: {}", path.display(), e),
        }
    }

    (output, row_count)
}

/// Compute per-year P&L summary from all ledger files.
/// Returns `Vec<(year_str, gross_pnl, fees, net_pnl, trade_count, wins, losses)>`.
pub fn yearly_summary() -> Vec<(String, f64, f64, f64, usize, usize, usize)> {
    use std::collections::BTreeMap;

    let current_year = chrono::Utc::now()
        .format("%Y")
        .to_string()
        .parse::<i32>()
        .unwrap_or(2025);
    // year → (gross, fees, net, count, wins, losses)
    let mut map: BTreeMap<String, (f64, f64, f64, usize, usize, usize)> = BTreeMap::new();

    for year in 2024..=(current_year + 1) {
        let path = ledger_path(year);
        if !path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines().skip(1) {
            if line.is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split(',').collect();
            if cols.len() < 15 {
                continue;
            }
            let gross: f64 = cols[10].parse().unwrap_or(0.0);
            let fees: f64 = cols[11].parse().unwrap_or(0.0);
            let net: f64 = cols[12].parse().unwrap_or(0.0);
            let year_key = cols[0].get(..4).unwrap_or("????").to_string();
            let entry = map.entry(year_key).or_default();
            entry.0 += gross;
            entry.1 += fees;
            entry.2 += net;
            entry.3 += 1;
            if net >= 0.0 {
                entry.4 += 1;
            } else {
                entry.5 += 1;
            }
        }
    }

    map.into_iter()
        .map(|(y, (g, f, n, c, w, l))| (y, g, f, n, c, w, l))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_dashboard::ClosedTrade;

    fn sample_trade(pnl: f64, year: &str) -> ClosedTrade {
        ClosedTrade {
            symbol: "SOL".to_string(),
            side: "LONG".to_string(),
            entry: 100.0,
            exit: if pnl > 0.0 { 110.0 } else { 90.0 },
            pnl,
            pnl_pct: pnl,
            reason: "TakeProfit".to_string(),
            closed_at: format!("{}-06-15T12:00:00Z", year),
            entry_time: format!("{}-06-14T12:00:00Z", year),
            quantity: 1.0,
            size_usd: 100.0,
            leverage: 2.0,
            fees_est: 0.0,
            breakdown: None,
            note: None,
            venue: "Hyperliquid Perps".to_string(),
        }
    }

    #[test]
    fn estimate_fees_basic() {
        // 100 USD × 2× leverage × 0.075% × 2 legs = 0.30
        let f = estimate_fees(100.0, 2.0);
        assert!((f - 0.30).abs() < 0.001, "fees={}", f);
    }

    #[test]
    fn estimate_fees_no_leverage() {
        // 1× leverage
        let f = estimate_fees(100.0, 1.0);
        assert!((f - 0.15).abs() < 0.001, "fees={}", f);
    }

    #[test]
    fn append_creates_file() {
        let t = sample_trade(10.0, "2099");
        append(&t);
        let path = ledger_path(2099);
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("SOL"));
        assert!(content.contains("LONG"));
        // cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn append_multiple_rows_one_header() {
        // Remove any leftover file from a previous failed run so the header
        // is always written fresh — idempotent, ignores errors if file absent.
        let _ = std::fs::remove_file(ledger_path(2098));
        let t1 = sample_trade(5.0, "2098");
        let t2 = sample_trade(-3.0, "2098");
        append(&t1);
        append(&t2);
        let path = ledger_path(2098);
        let content = std::fs::read_to_string(&path).unwrap();
        let header_count = content.lines().filter(|l| l.starts_with("date,")).count();
        assert_eq!(header_count, 1, "Should have exactly one header");
        let data_rows = content
            .lines()
            .filter(|l| !l.starts_with("date,") && !l.is_empty())
            .count();
        assert_eq!(data_rows, 2, "Should have 2 data rows");
        let _ = std::fs::remove_file(&path);
    }
}
