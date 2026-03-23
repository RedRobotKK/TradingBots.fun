#![allow(dead_code)]

use crate::ai_feedback::GuardrailFeedback;
use crate::pattern_insights;
use crate::trade_log::TradeEvent;
use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub(crate) const REPORT_DIR: &str = "reports";
pub(crate) const GUARDRAIL_LOG: &str = "logs/ai_guardrail_feedback.jsonl";
pub(crate) const MARKET_LOG_PREFIX: &str = "logs/trading_";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolSummary {
    pub symbol: String,
    pub win_count: usize,
    pub loss_count: usize,
    pub total_pnl: f64,
    pub avg_pnl: f64,
    pub avg_guardrail_score: Option<f64>,
    pub avg_alignment_pct: Option<f64>,
    pub top_win_breakdowns: Vec<(String, usize)>,
    pub top_loss_breakdowns: Vec<(String, usize)>,
    pub top_win_contexts: Vec<(String, usize)>,
    pub top_loss_contexts: Vec<(String, usize)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableRow {
    pub symbol: String,
    pub breakdown: String,
    pub occurrences: usize,
    pub avg_guardrail_score: Option<f64>,
    pub avg_alignment_pct: Option<f64>,
    pub avg_pnl: f64,
    pub context: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportSummary {
    pub generated_at: DateTime<Utc>,
    pub date: NaiveDate,
    pub report_hash: String,
    pub symbol_summaries: Vec<SymbolSummary>,
    pub win_signature_table: Vec<TableRow>,
    pub loss_warning_table: Vec<TableRow>,
    pub daily_winner: Option<(String, f64)>,
    pub daily_loser: Option<(String, f64)>,
}

impl ReportSummary {
    fn hash(&self) -> String {
        let payload = serde_json::to_string(&self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    }
}

pub struct ReportRequest {
    pub date: NaiveDate,
}

pub fn generate_report(req: &ReportRequest) -> Result<ReportSummary> {
    let guardrail_entries = load_guardrail_entries(GUARDRAIL_LOG)?;
    let trade_events = load_trade_exits(req)?;

    let mut guardrail_by_symbol: HashMap<String, Vec<(DateTime<Utc>, GuardrailFeedback)>> =
        HashMap::new();
    for entry in guardrail_entries {
        guardrail_by_symbol
            .entry(entry.symbol.clone())
            .or_default()
            .push((entry_ts(&entry.ts)?, entry));
    }
    for list in guardrail_by_symbol.values_mut() {
        list.sort_by_key(|(ts, _)| *ts);
    }

    let mut accumulators: HashMap<String, SymbolAccumulator> = HashMap::new();
    let mut winner: Option<(String, f64)> = None;
    let mut loser: Option<(String, f64)> = None;

    for (ts, exit) in trade_events {
        let symbol = exit.symbol.clone();
        let entry = guardrail_for_exit(&guardrail_by_symbol, &symbol, ts);
        let is_win = exit.pnl_usd >= 0.0;
        let acc = accumulators.entry(symbol.clone()).or_default();
        acc.total_pnl += exit.pnl_usd;
        if is_win {
            acc.win_count += 1;
        } else {
            acc.loss_count += 1;
        }
        if let Some(guardrail) = entry {
            acc.guardrail_score_sum += guardrail.guardrail_score;
            acc.guardrail_score_count += 1;
            acc.alignment_pct_sum += guardrail.signal_alignment_pct;
            let breakdown = guardrail.signal_breakdown.clone();
            if is_win {
                *acc.win_breakdowns.entry(breakdown.clone()).or_insert(0) += 1;
                *acc.win_contexts
                    .entry(context_signature(guardrail))
                    .or_insert(0) += 1;
            } else {
                *acc.loss_breakdowns.entry(breakdown.clone()).or_insert(0) += 1;
                *acc.loss_contexts
                    .entry(context_signature(guardrail))
                    .or_insert(0) += 1;
            }
        }
        if winner
            .as_ref()
            .map(|(_, pnl)| exit.pnl_usd > *pnl)
            .unwrap_or(true)
        {
            winner = Some((symbol.clone(), exit.pnl_usd));
        }
        if loser
            .as_ref()
            .map(|(_, pnl)| exit.pnl_usd < *pnl)
            .unwrap_or(true)
        {
            loser = Some((symbol.clone(), exit.pnl_usd));
        }
    }

    let mut summaries: Vec<SymbolSummary> = accumulators
        .into_iter()
        .map(|(symbol, acc)| acc.into_summary(symbol))
        .collect();
    summaries.sort_by(|a, b| {
        b.total_pnl
            .partial_cmp(&a.total_pnl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let win_table = gather_table_rows(&summaries, true);
    let loss_table = gather_table_rows(&summaries, false);
    let mut summary = ReportSummary {
        generated_at: Utc::now(),
        date: req.date,
        report_hash: String::new(),
        symbol_summaries: summaries,
        win_signature_table: win_table,
        loss_warning_table: loss_table,
        daily_winner: winner,
        daily_loser: loser,
    };
    summary.report_hash = summary.hash();
    Ok(summary)
}

fn gather_table_rows(summaries: &[SymbolSummary], wins: bool) -> Vec<TableRow> {
    let mut rows = Vec::new();
    for summary in summaries {
        let breakdowns = if wins {
            &summary.top_win_breakdowns
        } else {
            &summary.top_loss_breakdowns
        };
        let contexts = if wins {
            &summary.top_win_contexts
        } else {
            &summary.top_loss_contexts
        };
        for (breakdown, count) in breakdowns.iter().take(3) {
            rows.push(TableRow {
                symbol: summary.symbol.clone(),
                breakdown: breakdown.clone(),
                occurrences: *count,
                avg_guardrail_score: summary.avg_guardrail_score,
                avg_alignment_pct: summary.avg_alignment_pct,
                avg_pnl: summary.avg_pnl,
                context: contexts
                    .iter()
                    .map(|(ctx, occ)| format!("{} ({})", ctx, occ))
                    .take(1)
                    .collect::<Vec<_>>()
                    .join("; "),
            });
        }
    }
    rows
}

pub(crate) fn context_signature(entry: &GuardrailFeedback) -> String {
    format!(
        "Fund {} | Order {} | Cross {} ",
        entry.funding_phase, entry.order_flow_snapshot, entry.cross_exchange_snapshot
    )
}

pub(crate) fn guardrail_for_exit<'a>(
    grouping: &'a HashMap<String, Vec<(DateTime<Utc>, GuardrailFeedback)>>,
    symbol: &str,
    ts: DateTime<Utc>,
) -> Option<&'a GuardrailFeedback> {
    grouping.get(symbol).and_then(|list| {
        list.iter()
            .rev()
            .find(|(entry_ts, _)| *entry_ts <= ts)
            .map(|(_, entry)| entry)
    })
}

pub(crate) fn entry_ts(src: &str) -> Result<DateTime<Utc>> {
    let dt = DateTime::parse_from_rfc3339(src)?;
    Ok(dt.with_timezone(&Utc))
}

pub(crate) fn load_guardrail_entries(path: &str) -> Result<Vec<GuardrailFeedback>> {
    let file = fs::File::open(path)?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        entries.push(serde_json::from_str(&line)?);
    }
    Ok(entries)
}

pub(crate) fn load_trade_exits(req: &ReportRequest) -> Result<Vec<(DateTime<Utc>, TradeExit)>> {
    let fname = format!("{}{}.jsonl", MARKET_LOG_PREFIX, req.date);
    let file = fs::File::open(&fname)?;
    let mut exits = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: TradeEvent = serde_json::from_str(&line)?;
        if let TradeEvent::TradeExit {
            ts,
            symbol,
            side,
            pnl_usd,
            pnl_pct,
            r_multiple,
            reason,
            cycles_held,
            minutes_held,
            dca_count,
            ..
        } = event
        {
            let ts = entry_ts(&ts)?;
            exits.push((
                ts,
                TradeExit {
                    symbol,
                    side,
                    pnl_usd,
                    pnl_pct,
                    reason,
                    r_multiple,
                    cycles_held,
                    minutes_held,
                    dca_count,
                },
            ));
        }
    }
    Ok(exits)
}

#[derive(Clone, Debug)]
pub(crate) struct TradeExit {
    pub symbol: String,
    pub side: String,
    pub pnl_usd: f64,
    pub pnl_pct: f64,
    pub reason: String,
    pub r_multiple: f64,
    pub cycles_held: u32,
    pub minutes_held: u32,
    pub dca_count: u8,
}

#[derive(Default)]
struct SymbolAccumulator {
    win_count: usize,
    loss_count: usize,
    total_pnl: f64,
    guardrail_score_sum: f64,
    guardrail_score_count: usize,
    alignment_pct_sum: f64,
    win_breakdowns: HashMap<String, usize>,
    loss_breakdowns: HashMap<String, usize>,
    win_contexts: HashMap<String, usize>,
    loss_contexts: HashMap<String, usize>,
}

impl SymbolAccumulator {
    fn into_summary(self, symbol: String) -> SymbolSummary {
        let guardrail_avg = if self.guardrail_score_count > 0 {
            Some(self.guardrail_score_sum / self.guardrail_score_count as f64)
        } else {
            None
        };
        let alignment_avg = if self.guardrail_score_count > 0 {
            Some(self.alignment_pct_sum / self.guardrail_score_count as f64)
        } else {
            None
        };
        let breakdowns = |map: HashMap<String, usize>| {
            let mut pairs: Vec<_> = map.into_iter().collect();
            pairs.sort_by_key(|(_, count)| Reverse(*count));
            pairs
        };
        let avg_pnl = if self.win_count + self.loss_count > 0 {
            self.total_pnl / (self.win_count + self.loss_count) as f64
        } else {
            0.0
        };
        SymbolSummary {
            symbol,
            win_count: self.win_count,
            loss_count: self.loss_count,
            total_pnl: self.total_pnl,
            avg_pnl,
            avg_guardrail_score: guardrail_avg,
            avg_alignment_pct: alignment_avg,
            top_win_breakdowns: breakdowns(self.win_breakdowns)
                .into_iter()
                .take(3)
                .collect(),
            top_loss_breakdowns: breakdowns(self.loss_breakdowns)
                .into_iter()
                .take(3)
                .collect(),
            top_win_contexts: breakdowns(self.win_contexts).into_iter().take(3).collect(),
            top_loss_contexts: breakdowns(self.loss_contexts).into_iter().take(3).collect(),
        }
    }
}

pub fn write_markdown(summary: &ReportSummary, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let file_name = format!("trade_journal_{}.md", summary.date);
    let path = dest.join(&file_name);
    let mut f = fs::File::create(&path)?;
    writeln!(f, "# Trade Journal {}", summary.date)?;
    writeln!(
        f,
        "Generated at {} | Report hash `{}`",
        summary.generated_at, summary.report_hash
    )?;
    writeln!(f, "## Daily win signatures")?;
    writeln!(
        f,
        "| Symbol | Breakdown | Context | Count | Avg Guardrail | Avg Alignment | Avg PnL |"
    )?;
    writeln!(f, "|---|---|---|---|---|---|---|")?;
    for row in &summary.win_signature_table {
        writeln!(
            f,
            "| {symbol} | {breakdown} | {context} | {count} | {guardrail:.2} | {align:.2}% | {pnl:.2} |",
            symbol = row.symbol,
            breakdown = row.breakdown,
            context = row.context,
            count = row.occurrences,
            guardrail = row.avg_guardrail_score.unwrap_or(0.0),
            align = row.avg_alignment_pct.unwrap_or(0.0) * 100.0,
            pnl = row.avg_pnl,
        )?;
    }
    writeln!(f, "\\n## Loss warnings")?;
    writeln!(
        f,
        "| Symbol | Breakdown | Context | Count | Avg Guardrail | Avg Alignment | Avg PnL |"
    )?;
    writeln!(f, "|---|---|---|---|---|---|---|")?;
    for row in &summary.loss_warning_table {
        writeln!(
            f,
            "| {symbol} | {breakdown} | {context} | {count} | {guardrail:.2} | {align:.2}% | {pnl:.2} |",
            symbol = row.symbol,
            breakdown = row.breakdown,
            context = row.context,
            count = row.occurrences,
            guardrail = row.avg_guardrail_score.unwrap_or(0.0),
            align = row.avg_alignment_pct.unwrap_or(0.0) * 100.0,
            pnl = row.avg_pnl,
        )?;
    }
    writeln!(f, "\\n## Daily winner / loser")?;
    if let Some((symbol, pnl)) = &summary.daily_winner {
        writeln!(f, "- Winner: {symbol} (${pnl:.2})")?;
    }
    if let Some((symbol, pnl)) = &summary.daily_loser {
        writeln!(f, "- Loser: {symbol} (${pnl:.2})")?;
    }
    Ok(())
}

pub fn persist_summary(summary: &ReportSummary) -> Result<()> {
    fs::create_dir_all(REPORT_DIR)?;
    let path = Path::new(REPORT_DIR).join("latest_report.json");
    fs::write(path, serde_json::to_string_pretty(summary)?)?;
    Ok(())
}

pub struct ReportRefreshResult {
    pub journal_path: PathBuf,
    pub pattern_json_path: PathBuf,
    pub pattern_md_path: PathBuf,
}

pub fn refresh_reports(date: NaiveDate) -> Result<ReportRefreshResult> {
    let req = ReportRequest { date };
    let summary = generate_report(&req)?;
    let reports_dir = Path::new(REPORT_DIR);
    write_markdown(&summary, reports_dir)?;
    persist_summary(&summary)?;
    let mut cache = QueryCache::load();
    cache.report_hash = summary.report_hash.clone();
    cache.updated_at = Utc::now();
    cache.save()?;
    let insights = pattern_insights::PatternInsights::build(date, &summary)?;
    let (json_path, md_path) = insights.persist()?;
    let mut pattern_cache = pattern_insights::PatternCache::load();
    pattern_cache.store(insights);
    pattern_cache.save()?;
    Ok(ReportRefreshResult {
        journal_path: reports_dir.join(format!("trade_journal_{}.md", date)),
        pattern_json_path: json_path,
        pattern_md_path: md_path,
    })
}

pub fn load_summary() -> Result<ReportSummary> {
    let path = Path::new(REPORT_DIR).join("latest_report.json");
    let data = fs::read_to_string(path)?;
    let summary: ReportSummary = serde_json::from_str(&data)?;
    Ok(summary)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryCache {
    pub updated_at: DateTime<Utc>,
    pub report_hash: String,
    pub entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub answer: String,
    pub timestamp: DateTime<Utc>,
}

impl QueryCache {
    pub fn load() -> Self {
        let path = Path::new(REPORT_DIR).join("report_query_cache.json");
        if let Ok(json) = fs::read_to_string(&path) {
            if let Ok(cache) = serde_json::from_str::<QueryCache>(&json) {
                return cache;
            }
        }
        QueryCache {
            updated_at: Utc::now(),
            report_hash: String::new(),
            entries: HashMap::new(),
        }
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(REPORT_DIR)?;
        let path = Path::new(REPORT_DIR).join("report_query_cache.json");
        let mut file = fs::File::create(path)?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    pub fn question_hash(question: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(question.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn lookup(&self, question: &str, report_hash: &str) -> Option<&CacheEntry> {
        if self.report_hash != report_hash {
            return None;
        }
        self.entries.get(&Self::question_hash(question))
    }

    pub fn store(&mut self, question: &str, report_hash: &str, answer: String) {
        self.report_hash = report_hash.to_string();
        let key = Self::question_hash(question);
        self.entries.insert(
            key,
            CacheEntry {
                answer,
                timestamp: Utc::now(),
            },
        );
        self.updated_at = Utc::now();
    }
}
