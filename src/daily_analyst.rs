//! Daily log analysis powered by Claude.
//!
//! # Purpose
//!
//! Reads a day's `trading_YYYY-MM-DD.jsonl` file, computes aggregate statistics,
//! then sends a rich structured prompt to Claude Sonnet asking for:
//!
//!   1. **Pattern analysis** — which signal combinations produced wins vs losses
//!   2. **Indicator calibration** — whether thresholds (RSI, ADX, confidence gates)
//!      should be tightened or relaxed based on the day's IC
//!   3. **Session/time patterns** — if certain UTC hours consistently outperform
//!   4. **Regime accuracy** — whether the ADX-based regime classifier was correct
//!   5. **Concrete parameter suggestions** — specific numeric changes to consider
//!
//! # Output
//!
//! Analysis is written to `logs/analysis_YYYY-MM-DD.md` as a markdown report
//! suitable for reading, version-controlling, or re-ingesting into a future
//! conversation.
//!
//! # Usage
//!
//! ```text
//! # Analyse yesterday automatically (called by midnight job):
//! analyse_day(logger, api_key).await
//!
//! # Analyse a specific date from CLI:
//! analyse_specific_day(log_path, output_path, api_key).await
//! ```

use anyhow::{anyhow, Result};
use log::{info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::trade_log::{SharedTradeLogger, TradeEvent, date_yesterday};

// ─────────────────────────── Claude API plumbing ─────────────────────────────

#[derive(Debug, Serialize)]
struct ClaudeRequest {
    model:      String,
    max_tokens: u32,
    system:     String,
    messages:   Vec<ClaudeMessage>,
}

#[derive(Debug, Serialize)]
struct ClaudeMessage {
    role:    String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
struct ClaudeContent {
    text: String,
}

// ─────────────────────────── Aggregate stats ─────────────────────────────────

/// Pre-computed statistics extracted from the JSONL log.
/// Passed to Claude as a compact structured context.
#[derive(Debug, Default, Serialize)]
struct DayStats {
    date:               String,
    total_cycles:       u64,
    total_decisions:    u64,
    total_skips:        u64,
    total_entries:      u64,
    total_exits:        u64,
    total_partials:     u64,
    total_dcas:         u64,
    cb_activations:     u64,

    // P&L
    day_pnl_usd:        f64,
    win_count:          u64,
    loss_count:         u64,
    win_rate:           f64,
    avg_win_usd:        f64,
    avg_loss_usd:       f64,
    largest_win_usd:    f64,
    largest_loss_usd:   f64,
    best_symbol:        String,
    worst_symbol:       String,

    // R-multiple distribution
    avg_r_at_close:     f64,
    r_above_2:          u64,   // exits that reached 2R
    r_above_4:          u64,   // exits that reached 4R
    stop_loss_count:    u64,
    time_exit_count:    u64,
    tp_count:           u64,

    // Indicator stats on winning vs losing entries
    wins_avg_confidence: f64,
    losses_avg_confidence: f64,
    wins_avg_adx:        f64,
    losses_avg_adx:      f64,
    wins_avg_rsi:        f64,
    losses_avg_rsi:      f64,
    wins_avg_volume_ratio: f64,
    losses_avg_volume_ratio: f64,
    wins_avg_atr_expansion: f64,
    losses_avg_atr_expansion: f64,

    // Regime breakdown
    trending_win_rate:  f64,
    neutral_win_rate:   f64,
    ranging_win_rate:   f64,
    trending_count:     u64,
    neutral_count:      u64,
    ranging_count:      u64,

    // Session (UTC hour) breakdown — best 3 hours
    best_hours:         Vec<HourStats>,
    worst_hours:        Vec<HourStats>,

    // Signal combination frequency (top 5 rationale tags)
    top_winning_signals: Vec<SignalFreq>,
    top_losing_signals:  Vec<SignalFreq>,

    // Average hold time
    avg_hold_minutes:   f64,
    max_hold_minutes:   u32,

    // Final portfolio state
    final_capital:      f64,
    start_capital:      f64,
    peak_equity_day:    f64,
    max_drawdown_day:   f64,
}

#[derive(Debug, Default, Serialize, Clone)]
struct HourStats {
    hour:      u8,
    entries:   u64,
    wins:      u64,
    win_rate:  f64,
    pnl:       f64,
}

#[derive(Debug, Default, Serialize, Clone)]
struct SignalFreq {
    tag:   String,
    count: u64,
    wins:  u64,
    ic:    f64,   // wins/count − 0.5 (information coefficient proxy)
}

// ─────────────────────────── JSONL parser ────────────────────────────────────

/// Read and parse a JSONL log file into a vec of `TradeEvent`.
fn load_events(path: &Path) -> Result<Vec<TradeEvent>> {
    if !path.exists() {
        anyhow::bail!("Log file not found: {}", path.display());
    }
    let file   = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() { continue; }
        match serde_json::from_str::<TradeEvent>(line) {
            Ok(ev) => events.push(ev),
            Err(e) => warn!("  Line {}: parse error — {}", i + 1, e),
        }
    }
    Ok(events)
}

// ─────────────────────────── Stats computation ───────────────────────────────

/// Compute aggregate `DayStats` from the raw event list.
fn compute_stats(events: &[TradeEvent], date: &str) -> DayStats {
    let mut s = DayStats {
        date: date.to_string(),
        ..Default::default()
    };

    // Per-symbol P&L accumulators
    let mut symbol_pnl: HashMap<String, f64> = HashMap::new();
    // Linked entry → exit by symbol (simplified: last entry per symbol)
    let mut pending_entries: HashMap<String, TradeEntry_> = HashMap::new();
    // Hour buckets
    let mut hour_stats: [HourStats; 24] = std::array::from_fn(|h| HourStats { hour: h as u8, ..Default::default() });
    // Signal tag accumulators: tag → (count, wins)
    let mut win_tags:  HashMap<String, (u64, u64)> = HashMap::new();
    let mut loss_tags: HashMap<String, (u64, u64)> = HashMap::new();

    let mut win_conf_sum = 0.0f64;
    let mut loss_conf_sum = 0.0f64;
    let _win_adx_sum  = 0.0f64;
    let _loss_adx_sum = 0.0f64;
    let _win_rsi_sum  = 0.0f64;
    let _loss_rsi_sum = 0.0f64;
    let _win_vol_sum  = 0.0f64;
    let _loss_vol_sum = 0.0f64;
    let _win_atr_sum  = 0.0f64;
    let _loss_atr_sum = 0.0f64;
    let _trending_wins = 0u64; let mut trending_total = 0u64;
    let _neutral_wins  = 0u64; let mut neutral_total  = 0u64;
    let _ranging_wins  = 0u64; let mut ranging_total  = 0u64;
    let mut r_sum   = 0.0f64;
    let mut hold_sum = 0u64;
    let mut all_wins_usd: Vec<f64> = Vec::new();
    let mut all_losses_usd: Vec<f64> = Vec::new();
    let mut peak_eq = 0.0f64;

    for ev in events {
        match ev {
            TradeEvent::CycleStart { cycle_number, free_capital, peak_equity, .. } => {
                s.total_cycles = *cycle_number;
                if *peak_equity > peak_eq { peak_eq = *peak_equity; }
                if s.start_capital == 0.0 { s.start_capital = *free_capital; }
                s.final_capital = *free_capital;
            }

            TradeEvent::Decision { action, skip_reason, regime, .. } => {
                s.total_decisions += 1;
                if action == "SKIP" || skip_reason.is_some() { s.total_skips += 1; }
                if action != "SKIP" {
                    match regime.as_str() {
                        "Trending" => trending_total += 1,
                        "Neutral"  => neutral_total  += 1,
                        "Ranging"  => ranging_total  += 1,
                        _ => {}
                    }
                }
            }

            TradeEvent::TradeEntry { symbol, side, entry_price, confidence, ts,
                                     stop_loss, take_profit, leverage, size_usd,
                                     r_risk_usd, rationale, in_circuit_breaker,
                                     portfolio_heat_pct, kelly_pct, .. } => {
                s.total_entries += 1;
                let hour = parse_hour(ts);
                if hour < 24 {
                    hour_stats[hour as usize].entries += 1;
                }
                pending_entries.insert(symbol.clone(), TradeEntry_ {
                    symbol:    symbol.clone(),
                    side:      side.clone(),
                    price:     *entry_price,
                    confidence: *confidence,
                    stop_loss: *stop_loss,
                    size_usd:  *size_usd,
                    hour,
                    rationale: rationale.clone(),
                });
                // Extract regime tag from rationale for signal analysis
                for tag in extract_tags(rationale) {
                    win_tags.entry(tag.clone()).or_default();
                    loss_tags.entry(tag.clone()).or_default();
                }
            }

            TradeEvent::TradeExit { symbol, pnl_usd, r_multiple, reason,
                                    minutes_held, cycles_held, .. } => {
                s.total_exits += 1;
                *symbol_pnl.entry(symbol.clone()).or_insert(0.0) += pnl_usd;
                s.day_pnl_usd += pnl_usd;
                r_sum += r_multiple;
                hold_sum += *minutes_held as u64;
                if *minutes_held > s.max_hold_minutes { s.max_hold_minutes = *minutes_held; }

                // R distribution
                if *r_multiple >= 4.0 { s.r_above_4 += 1; s.r_above_2 += 1; }
                else if *r_multiple >= 2.0 { s.r_above_2 += 1; }

                match reason.as_str() {
                    "StopLoss"   => s.stop_loss_count += 1,
                    "TakeProfit" => s.tp_count        += 1,
                    "TimeExit"   => s.time_exit_count += 1,
                    _ => {}
                }

                // Correlate with entry for indicator analysis
                if let Some(entry) = pending_entries.remove(symbol) {
                    let won = *pnl_usd > 0.0;
                    if won {
                        s.win_count   += 1;
                        all_wins_usd.push(*pnl_usd);
                        win_conf_sum  += entry.confidence;
                        if entry.hour < 24 { hour_stats[entry.hour as usize].wins += 1; hour_stats[entry.hour as usize].pnl += pnl_usd; }
                        for tag in extract_tags(&entry.rationale) {
                            let e = win_tags.entry(tag).or_insert((0,0));
                            e.0 += 1; e.1 += 1;
                        }
                    } else {
                        s.loss_count  += 1;
                        all_losses_usd.push(*pnl_usd);
                        loss_conf_sum += entry.confidence;
                        if entry.hour < 24 { hour_stats[entry.hour as usize].pnl += pnl_usd; }
                        for tag in extract_tags(&entry.rationale) {
                            let e = loss_tags.entry(tag).or_insert((0,0));
                            e.0 += 1;
                        }
                    }
                }
            }

            TradeEvent::TradePartial { .. }   => s.total_partials += 1,
            TradeEvent::TradeDca    { .. }    => s.total_dcas     += 1,
            TradeEvent::CircuitBreaker { activated, .. } => {
                if *activated { s.cb_activations += 1; }
            }

            TradeEvent::MetricsSnapshot { capital, .. } => {
                s.final_capital = *capital;
            }

            _ => {}
        }
    }

    // Finalise rates
    let total_closed = s.win_count + s.loss_count;
    s.win_rate = if total_closed > 0 { s.win_count as f64 / total_closed as f64 } else { 0.0 };
    s.avg_r_at_close = if total_closed > 0 { r_sum / total_closed as f64 } else { 0.0 };
    s.avg_hold_minutes = if total_closed > 0 { hold_sum as f64 / total_closed as f64 } else { 0.0 };

    if s.win_count > 0 {
        s.wins_avg_confidence = win_conf_sum / s.win_count as f64;
        s.avg_win_usd  = all_wins_usd.iter().sum::<f64>() / s.win_count as f64;
        s.largest_win_usd = all_wins_usd.iter().cloned().fold(0.0_f64, f64::max);
    }
    if s.loss_count > 0 {
        s.losses_avg_confidence = loss_conf_sum / s.loss_count as f64;
        s.avg_loss_usd  = all_losses_usd.iter().sum::<f64>() / s.loss_count as f64;
        s.largest_loss_usd = all_losses_usd.iter().cloned().fold(0.0_f64, f64::min);
    }

    // Best / worst symbol
    if let Some((sym, pnl)) = symbol_pnl.iter().max_by(|a,b| a.1.partial_cmp(b.1).unwrap()) {
        s.best_symbol = sym.clone(); s.day_pnl_usd = *pnl + s.day_pnl_usd; // approximate
    }
    if let Some((sym, _)) = symbol_pnl.iter().min_by(|a,b| a.1.partial_cmp(b.1).unwrap()) {
        s.worst_symbol = sym.clone();
    }

    // Hour win-rates
    for h in &mut hour_stats {
        if h.entries > 0 {
            h.win_rate = h.wins as f64 / h.entries as f64;
        }
    }
    let mut sorted_hours: Vec<HourStats> = hour_stats.iter().filter(|h| h.entries > 0).cloned().collect();
    sorted_hours.sort_by(|a,b| b.win_rate.partial_cmp(&a.win_rate).unwrap());
    s.best_hours  = sorted_hours.iter().take(3).cloned().collect();
    s.worst_hours = sorted_hours.iter().rev().take(3).cloned().collect();

    // Signal IC
    let to_sigfreq = |map: &HashMap<String, (u64,u64)>| -> Vec<SignalFreq> {
        let mut v: Vec<SignalFreq> = map.iter().map(|(tag,(count,wins))| SignalFreq {
            tag: tag.clone(),
            count: *count,
            wins:  *wins,
            ic:    if *count > 0 { *wins as f64 / *count as f64 - 0.5 } else { 0.0 },
        }).collect();
        v.sort_by(|a,b| b.ic.partial_cmp(&a.ic).unwrap());
        v.truncate(8);
        v
    };
    s.top_winning_signals = to_sigfreq(&win_tags);
    s.top_losing_signals  = to_sigfreq(&loss_tags);

    s.peak_equity_day = peak_eq;
    s.max_drawdown_day = if peak_eq > 0.0 && s.final_capital < peak_eq {
        (peak_eq - s.final_capital) / peak_eq * 100.0
    } else { 0.0 };

    s
}

// Small helper struct — Rust doesn't allow tuple structs as hashmap values easily
#[derive(Debug, Clone)]
struct TradeEntry_ {
    symbol:     String,
    side:       String,
    price:      f64,
    confidence: f64,
    stop_loss:  f64,
    size_usd:   f64,
    hour:       u8,
    rationale:  String,
}

fn parse_hour(ts: &str) -> u8 {
    // ts format: "2026-02-27T14:32:11.123Z"
    ts.get(11..13)
        .and_then(|s| s.parse().ok())
        .unwrap_or(25)
}

fn extract_tags(rationale: &str) -> Vec<String> {
    // Extract bracketed tags like [Trending/Session], 4H:RSI, ⚡ATR×, Δ
    let mut tags = Vec::new();
    for word in rationale.split_whitespace() {
        let w = word.trim_matches(|c: char| "[](),".contains(c));
        if w.starts_with('[') || w.contains(':') || w.starts_with('⚡') || w.starts_with('Δ') {
            tags.push(w.to_string());
        }
    }
    // Also extract regime from brackets
    if rationale.contains("Trending") { tags.push("Trending".to_string()); }
    if rationale.contains("Ranging")  { tags.push("Ranging".to_string());  }
    if rationale.contains("Neutral")  { tags.push("Neutral".to_string());  }
    if rationale.contains("4H:RSI")   { tags.push("4H_MTF".to_string());   }
    if rationale.contains("⚡ATR")    { tags.push("ATR_Expansion".to_string()); }
    tags.dedup();
    tags
}

// ─────────────────────────── Claude prompt builder ───────────────────────────

fn build_system_prompt() -> &'static str {
    r#"You are a quantitative trading analyst reviewing the daily performance log of an algorithmic cryptocurrency trading bot.

The bot trades perps on Hyperliquid using these signals:
- Wilder's RSI(14), Bollinger Bands(20), MACD(12,26,9), ATR(14), ADX(14), Z-score(20), EMA(8/21) cross, VWAP(24)
- Multi-timeframe: 1h signals confirmed by 4h RSI + Z-score (IC +0.03–0.05)
- ATR expansion override: ADX Ranging → Trending when atr_expansion_ratio > 1.5 (IC +0.02)
- Session filter: UTC hours weighted [00-06: 1.00, 06-12: 1.06, 12-18: 1.14, 18-24: 1.06]
- Funding delta boost: rapid funding rate change boosts confidence 1.30–1.60×
- Relative BTC performance: asset lagging BTC by >2% over 4h at high dominance adds +0.04 confidence
- Regime: ADX>27 Trending, 19-27 Neutral, <19 Ranging
- Entry gate: confidence ≥ 0.68 (Ranging: scaled up, Trending: direct entry)
- Position sizing: half-Kelly, capped at 18% per trade, 2% R-risk, 8% portfolio heat
- Exits: 1/3 at 2R, 1/3 at 4R, trailing stop from 1.5R; time exits after 2-4 hours of stagnation
- Circuit breaker: 0.35× size when drawdown > 8%

Your job: analyse the day's trading data and produce a concise, actionable markdown report.

Structure your response EXACTLY as follows (use these exact markdown headers):

## Summary
2-3 sentence overview of the day's performance and market conditions.

## Pattern Analysis
What indicator combinations correlated with wins vs losses? Be specific with numbers.

## Regime Performance
Which regime (Trending/Neutral/Ranging) performed best and worst? Were regime classifications accurate?

## Session Analysis
Which UTC hours had the best/worst results? Should the session multipliers be adjusted?

## Signal IC Analysis
Which signals added positive information coefficient? Which subtracted value?

## Threshold Recommendations
Specific numeric suggestions. For each suggestion, state: current value → proposed value, with rationale based on today's data. Format as a table.

## Risk Observations
Any concerning patterns in drawdowns, stop-loss frequency, position sizing, or circuit breaker activity.

## Priority Actions
Top 3 concrete parameter changes to consider, ranked by expected impact. Be direct — "increase ADX trending threshold from 27 to 30" not "consider adjusting ADX."

Be data-driven, reference specific numbers from the stats, and avoid vague generalities. If the sample size is too small for statistical confidence, say so."#
}

// ─────────────────────────── Main entry points ───────────────────────────────

/// Analyse yesterday's log (called automatically at midnight).
///
/// Writes the markdown report to `logs/analysis_YYYY-MM-DD.md`.
/// Non-fatal: logs errors rather than crashing the trading loop.
pub async fn analyse_day(logger: &SharedTradeLogger, api_key: &str) {
    let (log_path, out_path) = {
        let lg = logger.lock().await;
        let date = date_yesterday();
        let log  = lg.log_path_for(&date);
        let out  = lg.log_dir().join(format!("analysis_{}.md", date));
        (log, out)
    };

    match analyse_log_file(&log_path, &out_path, api_key).await {
        Ok(path) => info!("🤖 Daily analysis written: {}", path.display()),
        Err(e)   => warn!("⚠ Daily analysis failed: {}", e),
    }
}

/// Analyse a specific log file (used by the `--analyze` CLI flag).
///
/// `log_path` — path to the `.jsonl` file to analyse
/// `out_path` — where to write the markdown report
pub async fn analyse_log_file(
    log_path: &Path,
    out_path: &Path,
    api_key:  &str,
) -> Result<std::path::PathBuf> {
    info!("📖 Loading log: {}", log_path.display());
    let events = load_events(log_path)?;
    if events.is_empty() {
        anyhow::bail!("Log file is empty or has no parseable events");
    }
    info!("  {} events loaded", events.len());

    // Extract date from filename: logs/trading_YYYY-MM-DD.jsonl
    let date = log_path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("trading_"))
        .unwrap_or("unknown")
        .to_string();

    let stats = compute_stats(&events, &date);

    // Serialize stats to compact JSON for the prompt
    let stats_json = serde_json::to_string_pretty(&stats)
        .map_err(|e| anyhow!("Stats serialisation error: {}", e))?;

    // Count lines for a quick summary in the prompt header
    let line_count = events.len();
    let user_msg = format!(
        "Daily trading log analysis for **{}**\n\n\
         Total log events: {}\n\n\
         ## Aggregated Statistics (JSON)\n\
         ```json\n{}\n```\n\n\
         Please analyse this data and provide your structured report.",
        date, line_count, stats_json
    );

    info!("🤖 Sending {} chars to Claude Sonnet for analysis…", user_msg.len());

    let request = crate::ai_reviewer::build_claude_request(
        "claude-sonnet-4-6-20250929",
        4096,
        build_system_prompt(),
        &user_msg,
    );

    let client = Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key",         api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type",      "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow!("HTTP error: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body   = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Claude API {} — {}", status, &body[..body.len().min(400)]));
    }

    let claude_resp: ClaudeResponse = resp.json().await
        .map_err(|e| anyhow!("JSON decode: {}", e))?;

    let analysis = claude_resp.content
        .into_iter()
        .find(|c| !c.text.is_empty())
        .map(|c| c.text)
        .ok_or_else(|| anyhow!("Empty Claude response"))?;

    // Write markdown report
    let report = format!(
        "# RedRobot HedgeBot — Daily Trading Analysis\n\
         **Date:** {}\n\
         **Generated:** {}\n\
         **Log events:** {}\n\n\
         ---\n\n\
         {}\n\n\
         ---\n\n\
         ## Raw Statistics\n\
         <details>\n<summary>Click to expand JSON stats</summary>\n\n\
         ```json\n{}\n```\n\
         </details>\n",
        date,
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        line_count,
        analysis,
        stats_json,
    );

    std::fs::write(out_path, &report)
        .map_err(|e| anyhow!("Failed to write {}: {}", out_path.display(), e))?;

    info!("📄 Analysis saved: {}", out_path.display());
    info!("──────────────────────────────────────────");
    // Print the analysis summary to the log so it appears in VPS output
    for line in analysis.lines().take(20) {
        info!("  {}", line);
    }
    if analysis.lines().count() > 20 {
        info!("  … (see {} for full report)", out_path.display());
    }
    info!("──────────────────────────────────────────");

    Ok(out_path.to_path_buf())
}

// `log_dir()` accessor lives in trade_log.rs as a pub method on TradeLogger.
