#![allow(dead_code)]

use crate::ai_feedback::GuardrailFeedback;
use crate::reporting::{
    context_signature, entry_ts, guardrail_for_exit, load_guardrail_entries, load_trade_exits,
    ReportRequest, ReportSummary, TradeExit, GUARDRAIL_LOG, MARKET_LOG_PREFIX, REPORT_DIR,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PATTERN_CACHE_FILE: &str = "pattern_cache.json";
const PATTERN_JSON_PREFIX: &str = "pattern_insights_";
const PATTERN_MD_PREFIX: &str = "pattern_insights_";
const CACHE_TTL_MINS: i64 = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TradeOutcome {
    Win,
    Loss,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatternEntry {
    pub ts: DateTime<Utc>,
    pub symbol: String,
    pub side: String,
    pub outcome: TradeOutcome,
    pub pnl_usd: f64,
    pub pnl_pct: f64,
    pub reason: String,
    pub guardrail_score: f64,
    pub guardrail_components: Vec<String>,
    pub guardrail_note: Option<String>,
    pub guardrail_allowed: bool,
    pub recommendation: String,
    pub r_multiple: f64,
    pub hold_minutes: u64,
    pub dca_remaining: u8,
    pub signal_breakdown: String,
    pub signal_alignment_pct: f64,
    pub signal_summary: String,
    pub entry_confidence: f64,
    pub funding_phase: String,
    pub order_flow_snapshot: String,
    pub order_flow_confidence: f64,
    pub order_flow_direction: String,
    pub ob_sentiment: String,
    pub ob_adverse_cycles: u32,
    pub funding_rate: f64,
    pub funding_delta: f64,
    pub onchain_strength: f64,
    pub cex_premium_pct: f64,
    pub cex_mode: String,
    pub cross_exchange_snapshot: String,
    pub false_breakout: bool,
    pub momentum_stall: bool,
    pub context: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalComboSummary {
    pub breakdown: String,
    pub context: String,
    pub wins: usize,
    pub losses: usize,
    pub occurrences: usize,
    pub win_rate: f64,
    pub avg_guardrail_score: f64,
    pub avg_alignment_pct: f64,
    pub avg_pnl: f64,
    pub symbols: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatternInsights {
    pub generated_at: DateTime<Utc>,
    pub date: NaiveDate,
    pub guardrail_hash: String,
    pub trade_log_hash: String,
    pub report_summary: ReportSummary,
    pub entries: Vec<PatternEntry>,
    pub combo_stats: Vec<SignalComboSummary>,
    pub top_win_combos: Vec<SignalComboSummary>,
    pub top_loss_combos: Vec<SignalComboSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatternCache {
    pub updated_at: DateTime<Utc>,
    pub insights: Option<PatternInsights>,
}

impl PatternCache {
    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(json) = fs::read_to_string(&path) {
            if let Ok(cache) = serde_json::from_str::<PatternCache>(&json) {
                return cache;
            }
        }
        PatternCache {
            updated_at: Utc::now(),
            insights: None,
        }
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(REPORT_DIR)?;
        let path = Self::path();
        let mut file = File::create(path)?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    pub fn store(&mut self, insights: PatternInsights) {
        self.insights = Some(insights);
        self.updated_at = Utc::now();
    }

    pub fn latest(&self) -> Option<PatternInsights> {
        self.insights.clone()
    }

    pub fn needs_refresh(&self, guardrail_hash: &str, trade_log_hash: &str) -> bool {
        match &self.insights {
            Some(insights) => {
                insights.guardrail_hash != guardrail_hash
                    || insights.trade_log_hash != trade_log_hash
                    || (Utc::now() - self.updated_at) >= Duration::minutes(CACHE_TTL_MINS)
            }
            None => true,
        }
    }

    fn path() -> PathBuf {
        Path::new(REPORT_DIR).join(PATTERN_CACHE_FILE)
    }
}

impl PatternInsights {
    pub fn build(date: NaiveDate, summary: &ReportSummary) -> Result<Self> {
        let req = ReportRequest { date };
        let guardrails = load_guardrail_entries(GUARDRAIL_LOG)?;
        let trade_exits = load_trade_exits(&req)?;
        let guardrail_map = build_guardrail_map(&guardrails)?;
        let entries: Vec<PatternEntry> = trade_exits
            .into_iter()
            .map(|(ts, exit)| {
                let guardrail = guardrail_for_exit(&guardrail_map, &exit.symbol, ts);
                PatternEntry::from_exit(ts, exit, guardrail)
            })
            .collect();
        let combo_stats = summarize_combos(&entries);
        let guardrail_hash =
            file_hash(GUARDRAIL_LOG).context("failed to hash guardrail log for insights")?;
        let trade_path = format!("{}{}.jsonl", MARKET_LOG_PREFIX, date);
        let trade_log_hash =
            file_hash(&trade_path).with_context(|| format!("failed to hash {}", trade_path))?;
        let top_win_combos = combo_stats
            .iter()
            .filter(|combo| combo.wins > combo.losses)
            .take(3)
            .cloned()
            .collect();
        let top_loss_combos = combo_stats
            .iter()
            .filter(|combo| combo.losses >= combo.wins)
            .take(3)
            .cloned()
            .collect();
        Ok(PatternInsights {
            generated_at: Utc::now(),
            date,
            guardrail_hash,
            trade_log_hash,
            report_summary: summary.clone(),
            entries,
            combo_stats,
            top_win_combos,
            top_loss_combos,
        })
    }

    pub fn persist(&self) -> Result<(PathBuf, PathBuf)> {
        let json = self.persist_json()?;
        let md = self.persist_markdown()?;
        Ok((json, md))
    }

    pub fn persist_json(&self) -> Result<PathBuf> {
        fs::create_dir_all(REPORT_DIR)?;
        let path = Path::new(REPORT_DIR).join(format!("{}{}.json", PATTERN_JSON_PREFIX, self.date));
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }

    pub fn persist_markdown(&self) -> Result<PathBuf> {
        fs::create_dir_all(REPORT_DIR)?;
        let path = Path::new(REPORT_DIR).join(format!("{}{}.md", PATTERN_MD_PREFIX, self.date));
        let mut file = File::create(&path)?;
        writeln!(file, "# Pattern Insights {}", self.date)?;
        writeln!(
            file,
            "Generated at {} | Guardrail hash {} | Trade hash {}\n",
            self.generated_at, self.guardrail_hash, self.trade_log_hash
        )?;
        writeln!(file, "## Win signature combos")?;
        writeln!(
            file,
            "| Breakdown | Context | Wins | Losses | Win rate | Avg Guardrail | Avg Alignment | Avg PnL | Symbols |"
        )?;
        writeln!(file, "|---|---|---|---|---|---|---|---|")?;
        for combo in &self.top_win_combos {
            writeln!(
                file,
                "| {breakdown} | {context} | {wins} | {losses} | {win_rate:.0}% | {guardrail:.2} | {alignment:.2}% | {pnl:.2} | {symbols} |",
                breakdown = combo.breakdown,
                context = combo.context,
                wins = combo.wins,
                losses = combo.losses,
                win_rate = combo.win_rate * 100.0,
                guardrail = combo.avg_guardrail_score,
                alignment = combo.avg_alignment_pct * 100.0,
                pnl = combo.avg_pnl,
                symbols = combo.symbols.join(", "),
            )?;
        }
        writeln!(file, "\n## Loss warning combos")?;
        writeln!(
            file,
            "| Breakdown | Context | Wins | Losses | Win rate | Avg Guardrail | Avg Alignment | Avg PnL | Symbols |"
        )?;
        writeln!(file, "|---|---|---|---|---|---|---|---|")?;
        for combo in &self.top_loss_combos {
            writeln!(
                file,
                "| {breakdown} | {context} | {wins} | {losses} | {win_rate:.0}% | {guardrail:.2} | {alignment:.2}% | {pnl:.2} | {symbols} |", 
                breakdown = combo.breakdown,
                context = combo.context,
                wins = combo.wins,
                losses = combo.losses,
                win_rate = combo.win_rate * 100.0,
                guardrail = combo.avg_guardrail_score,
                alignment = combo.avg_alignment_pct * 100.0,
                pnl = combo.avg_pnl,
                symbols = combo.symbols.join(", "),
            )?;
        }
        writeln!(file, "\n## Recent guardrail exits")?;
        writeln!(
            file,
            "| Time (UTC) | Symbol | PnL | Guardrail | Alignment | Recommendation | Context |"
        )?;
        writeln!(file, "|---|---|---|---|---|---|---|")?;
        for entry in self.entries.iter().rev().take(8) {
            writeln!(
                file,
                "| {time} | {symbol} | ${pnl:.2} | {score:.2} | {align:.2}% | {rec} | {context} |",
                time = entry.ts.format("%Y-%m-%d %H:%M:%S"),
                symbol = entry.symbol,
                pnl = entry.pnl_usd,
                score = entry.guardrail_score,
                align = entry.signal_alignment_pct * 100.0,
                rec = entry.recommendation,
                context = entry.context,
            )?;
        }
        Ok(path)
    }
}

impl PatternEntry {
    fn from_exit(
        ts: DateTime<Utc>,
        exit: TradeExit,
        guardrail: Option<&GuardrailFeedback>,
    ) -> Self {
        let (
            guardrail_score,
            components,
            note,
            allowed,
            recommendation,
            signal_breakdown,
            alignment_pct,
            summary,
            entry_confidence,
            funding_phase,
            order_flow_snapshot,
            order_flow_confidence,
            order_flow_direction,
            ob_sentiment,
            ob_adverse_cycles,
            funding_rate,
            funding_delta,
            onchain_strength,
            cex_premium_pct,
            cex_mode,
            cross_exchange_snapshot,
            false_breakout,
            momentum_stall,
            context,
        ) = if let Some(g) = guardrail {
            (
                g.guardrail_score,
                g.guardrail_components.clone(),
                g.guardrail_note.clone(),
                g.guardrail_allowed,
                g.recommendation.clone(),
                g.signal_breakdown.clone(),
                g.signal_alignment_pct,
                g.signal_summary.clone(),
                g.entry_confidence,
                g.funding_phase.clone(),
                g.order_flow_snapshot.clone(),
                g.order_flow_confidence,
                g.order_flow_direction.clone(),
                g.ob_sentiment.clone(),
                g.ob_adverse_cycles,
                g.funding_rate,
                g.funding_delta,
                g.onchain_strength,
                g.cex_premium_pct,
                g.cex_mode.clone(),
                g.cross_exchange_snapshot.clone(),
                g.false_breakout,
                g.momentum_stall,
                context_signature(g),
            )
        } else {
            (
                0.0,
                Vec::new(),
                None,
                true,
                String::new(),
                "".into(),
                0.0,
                "".into(),
                0.0,
                "".into(),
                "".into(),
                0.0,
                "".into(),
                "".into(),
                0,
                0.0,
                0.0,
                0.0,
                0.0,
                "".into(),
                "".into(),
                false,
                false,
                "(<no guardrail yet>)".into(),
            )
        };
        PatternEntry {
            ts,
            symbol: exit.symbol,
            side: exit.side,
            outcome: if exit.pnl_usd >= 0.0 {
                TradeOutcome::Win
            } else {
                TradeOutcome::Loss
            },
            pnl_usd: exit.pnl_usd,
            pnl_pct: exit.pnl_pct,
            reason: exit.reason,
            guardrail_score,
            guardrail_components: components,
            guardrail_note: note,
            guardrail_allowed: allowed,
            recommendation,
            r_multiple: exit.r_multiple,
            hold_minutes: exit.minutes_held.into(),
            dca_remaining: exit.dca_count,
            signal_breakdown,
            signal_alignment_pct: alignment_pct,
            signal_summary: summary,
            entry_confidence,
            funding_phase,
            order_flow_snapshot,
            order_flow_confidence,
            order_flow_direction,
            ob_sentiment,
            ob_adverse_cycles,
            funding_rate,
            funding_delta,
            onchain_strength,
            cex_premium_pct,
            cex_mode,
            cross_exchange_snapshot,
            false_breakout,
            momentum_stall,
            context,
        }
    }
}

fn build_guardrail_map(
    entries: &[GuardrailFeedback],
) -> Result<HashMap<String, Vec<(DateTime<Utc>, GuardrailFeedback)>>> {
    let mut map: HashMap<String, Vec<(DateTime<Utc>, GuardrailFeedback)>> = HashMap::new();
    for entry in entries {
        let parsed = entry_ts(&entry.ts)?;
        map.entry(entry.symbol.clone())
            .or_default()
            .push((parsed, entry.clone()));
    }
    for list in map.values_mut() {
        list.sort_by_key(|(ts, _)| *ts);
    }
    Ok(map)
}

fn summarize_combos(entries: &[PatternEntry]) -> Vec<SignalComboSummary> {
    #[derive(Default)]
    struct ComboAccum {
        wins: usize,
        losses: usize,
        occurrences: usize,
        guardrail_score_sum: f64,
        alignment_sum: f64,
        pnl_sum: f64,
        symbols: Vec<String>,
    }

    let mut combos: HashMap<(String, String), ComboAccum> = HashMap::new();
    for entry in entries {
        let key = (entry.signal_breakdown.clone(), entry.context.clone());
        let acc = combos.entry(key).or_insert_with(ComboAccum::default);
        acc.occurrences += 1;
        if let TradeOutcome::Win = entry.outcome {
            acc.wins += 1;
        } else {
            acc.losses += 1;
        }
        acc.guardrail_score_sum += entry.guardrail_score;
        acc.alignment_sum += entry.signal_alignment_pct;
        acc.pnl_sum += entry.pnl_usd;
        if !acc.symbols.contains(&entry.symbol) && acc.symbols.len() < 3 {
            acc.symbols.push(entry.symbol.clone());
        }
    }

    let mut summary: Vec<SignalComboSummary> = combos
        .into_iter()
        .map(|((breakdown, context), acc)| {
            let occurrences = acc.occurrences.max(1);
            SignalComboSummary {
                breakdown,
                context,
                wins: acc.wins,
                losses: acc.losses,
                occurrences,
                win_rate: acc.wins as f64 / occurrences as f64,
                avg_guardrail_score: acc.guardrail_score_sum / occurrences as f64,
                avg_alignment_pct: acc.alignment_sum / occurrences as f64,
                avg_pnl: acc.pnl_sum / occurrences as f64,
                symbols: acc.symbols,
            }
        })
        .collect();
    summary.sort_by(|a, b| b.occurrences.cmp(&a.occurrences));
    summary
}

fn file_hash(path: impl AsRef<Path>) -> Result<String> {
    let mut file = File::open(path.as_ref())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_needs_refresh_on_hash_change() {
        let mut cache = PatternCache {
            updated_at: Utc::now(),
            insights: Some(PatternInsights {
                generated_at: Utc::now(),
                date: Utc::now().date_naive(),
                guardrail_hash: "old".into(),
                trade_log_hash: "old".into(),
                report_summary: ReportSummary {
                    generated_at: Utc::now(),
                    date: Utc::now().date_naive(),
                    report_hash: String::new(),
                    symbol_summaries: vec![],
                    win_signature_table: vec![],
                    loss_warning_table: vec![],
                    daily_winner: None,
                    daily_loser: None,
                },
                entries: vec![],
                combo_stats: vec![],
                top_win_combos: vec![],
                top_loss_combos: vec![],
            }),
        };
        assert!(
            cache.needs_refresh("new", "new"),
            "hash mismatch should trigger refresh"
        );
        cache.insights.as_mut().unwrap().guardrail_hash = "new".into();
        cache.insights.as_mut().unwrap().trade_log_hash = "new".into();
        cache.updated_at = Utc::now() - Duration::minutes(CACHE_TTL_MINS + 1);
        assert!(
            cache.needs_refresh("new", "new"),
            "ttl expiry should trigger refresh"
        );
    }
}
