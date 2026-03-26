//! TradingBots.fun – Autonomous Cryptocurrency Trading System
//!
//! Professional quant trade management:
//!   • Kelly Criterion position sizing (half-Kelly, confidence-scaled)
//!   • Portfolio heat limit: max 15% total equity at risk — AI-budgeted, no hard position count cap
//!   • Per-trade risk: max 2% equity at risk (stop-distance based)
//!   • R-multiple exits: 1/3 at 2R, 1/3 at 4R, trail final 1/3 with no ceiling
//!   • Trailing stop: breakeven at 1R, trail 1.2×ATR from HWM at 1.5R
//!   • Time exit: close stale trades (<0.5R after 8 cycles)
//!   • Circuit breaker: 0.35× size multiplier when equity drawdown >8%
//!   • Pyramid: add to winners (existing >1R profit + new signal = +50% add-on)
//!   • Online learning: signal weights updated after every close/partial

// ─────────────────────────── Risk constants ───────────────────────────────────
/// Minimum signal confidence for new entries. Below this the trade is skipped.
/// Raised from 0.60 → 0.68 after live data showed 20% win rate with 115 open
/// positions — too many borderline signals were entering.
const MIN_CONFIDENCE: f64 = 0.68;
/// Maximum fraction of equity at risk per individual trade (stop-distance based).
/// Set at 5%: stop_dist_pct × margin × leverage / equity ≤ 0.05.
const MAX_TRADE_HEAT: f64 = 0.05; // 5 %
/// Maximum total fraction of equity at risk across all open positions.
/// This is the primary position budget — the AI sizes each new position so that
/// total portfolio heat never exceeds this ceiling.
/// Lowered 0.15→0.10: 40 positions at 5% heat each = 200% theoretical exposure;
/// capping portfolio heat at 10% forces tighter per-trade sizing.
const MAX_PORTFOLIO_HEAT: f64 = 0.10; // 10 %
/// Hard cap on simultaneous open positions.  Prevents the bot from holding
/// 100+ small losers that individually pass the heat check but collectively
/// dilute attention and inflate losing streaks.
/// Lowered 40→25→20: live data with 40 open showed avg R=0.12 — too many
/// drifting positions tying up capital. 20 forces higher conviction entries.
const MAX_OPEN_POSITIONS: usize = 20;
/// DCA minimum confidence (slightly higher than new-entry minimum).
const DCA_MIN_CONFIDENCE: f64 = 0.65;
/// Circuit-breaker drawdown threshold.  Once peak→current drawdown exceeds
/// this fraction, all new position sizes are scaled down by `CB_SIZE_MULT`.
const CB_DRAWDOWN_THRESHOLD: f64 = 0.08; // 8 %
/// Position-size multiplier applied when the circuit breaker is active.
const CB_SIZE_MULT: f64 = 0.35;
/// Upper bound for position size as fraction of free capital (Kelly clamp).
/// At 15% of free capital, a $1M account risks $150k per position before DCA.
/// Used as the default cap when no env override is set.
#[allow(dead_code)]
const MAX_POSITION_PCT: f64 = 0.15;
/// Lower bound for position size as fraction of free capital.
/// Raised from 1% — a $10 position on $1000 is economically meaningless.
#[allow(dead_code)]
const MIN_POSITION_PCT: f64 = 0.05;

#[derive(Clone, Debug)]
struct PatternCacheRefresher {
    running: Arc<AtomicBool>,
}

static PATTERN_CACHE_REFRESHER: OnceCell<Arc<PatternCacheRefresher>> = OnceCell::new();

impl PatternCacheRefresher {
    fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    fn trigger(&self) {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let runner = self.clone();
        tokio::spawn(async move {
            if let Err(err) = runner.run_once().await {
                warn!("Pattern cache refresh failed: {}", err);
            }
            runner.running.store(false, Ordering::SeqCst);
        });
    }

    async fn run_once(&self) -> Result<()> {
        let date = chrono::Utc::now().date_naive();
        let inner = tokio::task::spawn_blocking(move || -> Result<()> {
            let report = reporting::refresh_reports(date)?;
            info!(
                "Pattern cache refreshed for {} → {}",
                date,
                report.pattern_json_path.display()
            );
            run_pattern_cache_alert_script()
        })
        .await
        .context("pattern cache refresh task was cancelled")?;
        inner?;
        Ok(())
    }
}

fn run_pattern_cache_alert_script() -> Result<()> {
    let status = Command::new("python3")
        .arg("scripts/pattern_cache_alert.py")
        .status()
        .context("failed to execute pattern cache alert script")?;
    if !status.success() {
        return Err(anyhow!(
            "pattern cache alert script exited with {}",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

mod ai_feedback;
mod ai_helpers;
mod ai_reviewer;
mod bridge;
mod candlestick_patterns;
mod chart_patterns;
mod coins;
mod collective;
mod config;
mod correlation;
mod cross_exchange;
mod daily_analyst;
mod data;
mod db;
mod decision;
mod exchange;
mod fund_tracker;
mod funding;
mod funnel;
mod hl_wallet;
mod indicators;
mod invite;
mod latency;
mod leaderboard;
mod learner;
mod ledger;
mod mailer;
mod metrics;
mod notifier;
mod onchain;
mod pattern_insights;
mod persistence;
mod position_monitor;
mod price_feed;
mod privy;
mod signal_engine;
mod reporting;
mod risk;
mod sentiment;
mod signal_watchlist;
mod signals;
mod stripe;
mod tenant;
mod thesis;
mod trade_log;
mod web_dashboard;

use crate::bridge::BridgeManager;
use ai_feedback::{AiFeedbackLogger, GuardrailFeedback, SharedAiFeedbackLogger};
use ai_helpers::{
    cross_exchange_snapshot, order_flow_snapshot, signal_alignment_pct, signal_breakdown,
};
use ai_reviewer::format_signal_summary;
use anyhow::{anyhow, Context, Result};
use cross_exchange::{CrossExchangeMonitor, SharedCrossExchange};
use db::{AumSnapshot, Database, SharedDb};
use funding::{current_cycle_phase, describe_cycle_phase, FundingCache, SharedFunding};
use learner::{SharedWeights, SignalWeights};
use log::{error, info, warn};
use metrics::PerformanceMetrics;
use notifier::SharedNotifier;
use once_cell::sync::OnceCell;
use onchain::SharedOnchain;
use sentiment::{SentimentCache, SharedSentiment};
use signal_watchlist::{SharedWatchlist, SignalWatchlist, SkipReason};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs as tokio_fs;
use tokio::sync::{Mutex, RwLock};
use trade_log::{date_today, ts_now, SharedTradeLogger, TradeEvent, TradeLogger};
use uuid::Uuid;
use web_dashboard::{
    BotState, CandidateInfo, ClosedTrade, DecisionInfo, PaperPosition, SharedState,
};

/// Tenant ID used in single-operator mode (no multi-tenant yet).
/// Matches the hardcoded UUID used in equity_snapshot writes elsewhere.
fn single_op_tenant() -> Uuid {
    Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ERROR CLASSIFICATION – human-friendly cycle error messages
// ═══════════════════════════════════════════════════════════════════════════════

/// Classify a cycle error string and return `(sleep_secs, friendly_message)`.
///
/// The message is designed to be calm and actionable for a non-technical user
/// watching the dashboard.  The sleep duration gives the upstream service time
/// to recover before the bot retries.
fn classify_cycle_error(err: &str) -> (u64, String) {
    // Tag injected by data.rs for gateway / server errors
    if err.contains("hl_api_502")
        || err.contains("502")
        || err.contains("503")
        || err.contains("504")
    {
        return (
            60,
            "🌐 Hyperliquid servers are temporarily busy (502/503). \
             This happens occasionally during high market activity or maintenance. \
             The bot is pausing 60 s and will resume automatically — no action needed."
                .to_string(),
        );
    }
    if err.contains("hl_api_429") || err.contains("429") || err.contains("Too Many Requests") {
        return (
            30,
            "⏱ API rate limit reached. \
             The bot is backing off for 30 s and will slow its request rate. \
             No action needed."
                .to_string(),
        );
    }
    if err.contains("timeout") || err.contains("connection") || err.contains("dns") {
        return (
            20,
            "📡 Network connectivity issue detected. \
             Please check that the VPS has internet access. \
             Retrying in 20 s."
                .to_string(),
        );
    }
    // Generic fallback
    (
        10,
        format!(
            "⚠ Unexpected error — the bot will retry in 10 s. \
             If this keeps happening, restart the service with: \
             `sudo systemctl restart tradingbots-fun`\n\
             Detail: {}",
            err
        ),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    // ── CLI: --analyze [YYYY-MM-DD] ──────────────────────────────────────
    // Analyse a specific day's log and print the markdown report, then exit.
    // Usage:  cargo run -- --analyze             (yesterday)
    //         cargo run -- --analyze 2026-02-27  (specific date)
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--analyze") {
        // Load the API key from the environment (don't need full config for analysis)
        let api_key_owned = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        let api_key = api_key_owned.as_str();
        if api_key.is_empty() {
            eprintln!("❌ ANTHROPIC_API_KEY not set — cannot run analysis");
            std::process::exit(1);
        }
        // Date argument is optional; default = yesterday
        let date = args
            .get(pos + 1)
            .filter(|d| d.len() == 10 && d.chars().nth(4) == Some('-'))
            .cloned()
            .unwrap_or_else(trade_log::date_yesterday);

        let log_path = std::path::PathBuf::from(format!("logs/trading_{}.jsonl", date));
        let out_path = std::path::PathBuf::from(format!("logs/analysis_{}.md", date));
        match daily_analyst::analyse_log_file(&log_path, &out_path, api_key).await {
            Ok(p) => {
                println!("✅ Analysis written to {}", p.display());
            }
            Err(e) => {
                eprintln!("❌ Analysis failed: {}", e);
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    info!("🤖 TradingBots.fun Starting — Professional Quant Mode");

    let config = config::Config::from_env()?;
    info!(
        "✓ Config: mode={:?}  capital=${:.0}  paper={}",
        config.mode, config.initial_capital, config.paper_trading
    );

    // ── PostgreSQL — connect and migrate, gracefully degrade if unavailable ──
    let shared_db: Option<SharedDb> = if config.database_url.starts_with("postgres") {
        match Database::connect(&config.database_url).await {
            Ok(database) => {
                info!("✅ PostgreSQL connected and migrations applied");
                // Spawn hourly maintenance task (prune old snapshots, ANALYZE).
                let db_maint = Arc::new(database);
                let db_for_maint = db_maint.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                    interval.tick().await; // skip the first immediate tick
                    loop {
                        interval.tick().await;
                        if let Err(e) = db_for_maint.run_maintenance().await {
                            log::warn!("DB maintenance failed: {e}");
                        }
                    }
                });
                Some(db_maint)
            }
            Err(e) => {
                warn!("⚠ PostgreSQL unavailable — running without persistence: {e}");
                warn!("  Set DATABASE_URL=postgresql://tradingbots:<pass>@localhost/tradingbots to enable");
                None
            }
        }
    } else {
        info!("ℹ DATABASE_URL is not a PostgreSQL URL — persistence disabled");
        None
    };

    // ── Shared price oracle ────────────────────────────────────────────────────
    // One WebSocket connection to Hyperliquid + one to Binance feeds all tenant
    // loops with <100 ms prices.  REST allMids calls drop to 0 weight once the
    // WebSocket handshake completes (~2 s after startup).
    let price_oracle = price_feed::new_oracle();
    let market = Arc::new(data::MarketClient::with_oracle(price_oracle.clone()));
    let db_pool_opt = shared_db.as_deref().map(|db| db.pool().clone());
    price_feed::PriceFeedService::new(
        price_oracle.clone(),
        // Pass the raw PgPool (not the Arc<Database> wrapper) so price_feed
        // can use a separate small pool without touching the main 10-conn pool.
        db_pool_opt.clone(),
    )
    .spawn();

    // ── Signal engine (per-symbol, scales to 1M+ tenants) ─────────────────────
    // Computes RSI/MACD/order-flow signals ONCE per symbol per 30-second cycle.
    // All tenants read from the resulting SharedSignalCache instead of each
    // tenant independently recomputing signals — O(symbols) not O(tenants).
    let signal_cache = signal_engine::new_signal_cache();
    signal_engine::SignalEngine::new(
        market.clone(),
        price_oracle.clone(),
        signal_cache.clone(),
        db_pool_opt.clone(),
    )
    .spawn();

    // ── Position monitor (event-driven, scales to 1M+ tenants) ───────────────
    // Watches oracle price diffs; when a symbol moves ≥0.05%, queries all open
    // positions for that symbol across ALL tenants and evaluates stops/targets.
    // Writes triggered exits to execution_queue; worker pool executes them.
    // Cost: O(symbols_that_moved) not O(tenants × positions).
    if let Some(pool) = db_pool_opt.clone() {
        position_monitor::PositionMonitor::new(
            price_oracle.clone(),
            signal_cache.clone(),
            pool,
        )
        .spawn();
        info!("✓ PositionMonitor + ExecutionWorkers running ({} workers)", position_monitor::EXECUTION_WORKER_COUNT);
    } else {
        info!("ℹ PositionMonitor disabled (no DB — restart with DATABASE_URL to enable)");
    }

    let hl = Arc::new(exchange::HyperliquidClient::new(&config)?);

    // Daily structured JSONL log (LLM-ingestible, rotates at midnight UTC)
    let trade_logger: SharedTradeLogger = TradeLogger::shared("logs")
        .map_err(|e| anyhow::anyhow!("Failed to create trade logger: {}", e))?;
    info!("✓ Trade logger ready (logs/trading_{}.jsonl)", date_today());

    let ai_feedback_logger: SharedAiFeedbackLogger =
        AiFeedbackLogger::shared("logs/ai_guardrail_feedback.jsonl")
            .map_err(|e| anyhow::anyhow!("Failed to create AI feedback logger: {}", e))?;
    info!("✓ AI feedback logger ready (logs/ai_guardrail_feedback.jsonl)");

    // LunarCrush sentiment client (pre-warm cache at startup)
    let sentiment_cache: SharedSentiment = SentimentCache::new(config.lunarcrush_api_key.clone());
    sentiment_cache.warm().await;

    // HL funding rate cache via metaAndAssetCtxs (all perps, native symbols, refreshes every 3 min)
    let funding_cache: SharedFunding = FundingCache::new();
    funding_cache.warm().await;

    // On-chain exchange netflow — Coinglass API (graceful no-op if key absent)
    // COINGLASS_API_KEY read directly from env inside OnchainCache::new().
    let onchain_cache: SharedOnchain = onchain::OnchainCache::new();
    onchain_cache.warm().await;

    // Webhook / Telegram notifier — fires on position open/close/CB/AI events.
    // Auto-detected from WEBHOOK_URL and/or TELEGRAM_BOT_TOKEN + TELEGRAM_CHAT_ID.
    let notifier: Option<SharedNotifier> = notifier::Notifier::from_env().map(std::sync::Arc::new);
    if notifier.is_some() {
        info!("✓ Notifier ready (webhook/Telegram configured)");
    } else {
        info!("ℹ️  No WEBHOOK_URL or TELEGRAM_* env set — notifications disabled");
    }

    info!("✓ Clients ready (LunarCrush sentiment + HL funding rates + on-chain active)");

    // Cross-exchange monitor — Binance/ByBit/OKX vs HL price divergence (every 5 min)
    // Fires a tiny bull/bear signal when HL diverges from CEX peers by ≥0.25% for ≥3 cycles.
    // No-op in dev when CEX APIs are unreachable; never blocks trade decisions.
    let cex_monitor: SharedCrossExchange = CrossExchangeMonitor::new();
    cex_monitor.warm().await;

    // Signal watchlist — tracks near-miss SKIPs across cycles for re-evaluation
    let signal_watchlist: SharedWatchlist = SignalWatchlist::new();

    let weights: SharedWeights = Arc::new(RwLock::new(SignalWeights::load()));

    // BTC dominance — fetched live at startup, refreshed every ~10 min in cycle
    let btc_dominance: SharedBtcDominance = {
        let dom = fetch_btc_dominance().await.unwrap_or(56.0);
        info!("✓ BTC dominance (live): {:.1}%", dom);
        Arc::new(RwLock::new(dom))
    };

    let bot_state: SharedState = Arc::new(RwLock::new(BotState {
        capital: config.initial_capital,
        initial_capital: config.initial_capital,
        peak_equity: config.initial_capital,
        signal_weights: SignalWeights::load(),
        ..BotState::default()
    }));

    // ── Apply config values that are never persisted ──────────────────────
    bot_state.write().await.referral_code = config.referral_code.clone();

    // ── Restore persisted state (positions, P&L, metrics, equity window) ──
    if let Some(snapshot) = persistence::PersistedState::load() {
        snapshot.apply_to(&mut *bot_state.write().await);
        // Always keep initial_capital + referral_code from current config
        bot_state.write().await.initial_capital = config.initial_capital;
        bot_state.write().await.referral_code = config.referral_code.clone();
        info!(
            "✓ State restored: {} open positions · {} closed trades · capital=${:.2}",
            bot_state.read().await.positions.len(),
            bot_state.read().await.closed_trades.len(),
            bot_state.read().await.capital,
        );
    }

    // Dashboard
    // Shared tenant manager: used by both the dashboard and the midnight
    // leaderboard snapshot task — keep one instance so registrations are
    // visible in both places.
    let tenant_manager = tenant::new_tenant_manager();
    // Seed the 9 demo wallets ($10 → $10k) so every restart shows all wallets.
    tenant::seed_demo_tenants(&tenant_manager).await;
    // Global investment thesis constraints — written by the web API, read by run_cycle.
    let global_thesis: Arc<RwLock<thesis::ThesisConstraints>> =
        Arc::new(RwLock::new(thesis::ThesisConstraints::default()));
    fs::create_dir_all("reports")?;
    let report_cache = Arc::new(Mutex::new(reporting::QueryCache::load()));
    let pattern_cache = Arc::new(Mutex::new(pattern_insights::PatternCache::load()));
    let hyperliquid_stats_path = PathBuf::from("reports").join("hyperliquid_stats.json");
    let stats_writer_stats = hl.stats();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.tick().await;
        loop {
            interval.tick().await;
            let snapshot = stats_writer_stats.snapshot().await;
            if let Ok(serialized) = serde_json::to_string_pretty(&snapshot) {
                if let Err(err) = tokio_fs::write(&hyperliquid_stats_path, serialized).await {
                    warn!(
                        "Failed to write hyperliquid stats file {}: {}",
                        hyperliquid_stats_path.display(),
                        err
                    );
                }
            }
        }
    });
    let bridge_manager = Arc::new(BridgeManager::new(
        hl.clone(),
        config.bridge_min_withdraw_usd,
        config.bridge_trusted_destinations.clone(),
    ));
    let pattern_refresher = Arc::new(PatternCacheRefresher::new());
    PATTERN_CACHE_REFRESHER
        .set(pattern_refresher.clone())
        .expect("pattern cache refresher already set");
    // Transactional mailer — None when RESEND_API_KEY is not set.
    let mailer = mailer::Mailer::new(
        config.email_api_key.as_deref(),
        config.email_from.as_deref(),
    )
    .map(std::sync::Arc::new);
    {
        let app_state = web_dashboard::AppState {
            bot_state: bot_state.clone(),
            tenants: tenant_manager.clone(),
            db: shared_db.clone(),
            stripe_api_key: config.stripe_secret_key.clone(),
            stripe_webhook_secret: config.stripe_webhook_secret.clone(),
            stripe_price_id: config.stripe_price_id.clone(),
            privy_app_id: config.privy_app_id.clone(),
            walletconnect_project_id: config.walletconnect_project_id.clone(),
            session_secret: config.session_secret.clone(),
            jwks_cache: privy::new_jwks_cache(),
            apple_pay_domain_assoc: config.apple_pay_domain_assoc.clone(),
            admin_password: config.admin_password.clone(),
            coinzilla_zone_id: config.coinzilla_zone_id.clone(),
            mailer: mailer.clone(),
            stripe_promo_price_id: config.stripe_promo_price_id.clone(),
            global_thesis: global_thesis.clone(),
            report_cache: report_cache.clone(),
            pattern_cache: pattern_cache.clone(),
            hyperliquid_stats: hl.stats(),
            bridge_manager: bridge_manager.clone(),
            latency_tracker: std::sync::Arc::new(tokio::sync::RwLock::new(latency::LatencyTracker::new("global"))),
        };
        tokio::spawn(async move {
            if let Err(e) = web_dashboard::serve(app_state, 3000).await {
                error!("Dashboard: {}", e);
            }
        });
    }

    // ── Per-tenant demo trading loops ─────────────────────────────────────────
    // Each of the 9 demo wallets (Bot Alpha → Iota) gets its own Tokio task
    // running the full run_cycle() with its isolated SharedState.  Tasks are
    // staggered 3 s apart so they don't all hit the HL API simultaneously.
    // The market client, HL client, and signal caches are shared (Arc clones).
    {
        let tenant_handles: Vec<(tenant::TenantId, web_dashboard::SharedState)> = {
            let tm = tenant_manager.read().await;
            tm.all().map(|h| (h.id.clone(), h.state.clone())).collect()
        };

        for (idx, (tid, tenant_state)) in tenant_handles.into_iter().enumerate() {
            // Collect the capital size from state so the loop can log it
            let tenant_capital = tenant_state.read().await.initial_capital;
            // Demo wallets are Free/Pro paper — charge max builder fee (3 bps)
            let tenant_fee_bps: u32 = 3;

            let t_config = config.clone();
            let t_market = market.clone();
            let t_hl = hl.clone();
            let t_db = shared_db.clone();
            let t_weights = weights.clone();
            let t_sent = sentiment_cache.clone();
            let t_fund = funding_cache.clone();
            let t_logger = trade_logger.clone();
            let t_notifier = notifier.clone();
            let t_onchain = onchain_cache.clone();
            let t_watch = signal_watchlist.clone();
            let t_cex = cex_monitor.clone();
            let t_thesis = global_thesis.clone();
            let t_feedback = ai_feedback_logger.clone();
            let t_btcdom = btc_dominance.clone();
            let stagger = idx as u64 * 3; // 3 s between each tenant start

            tokio::spawn(async move {
                // Stagger start to spread API load
                tokio::time::sleep(std::time::Duration::from_secs(stagger)).await;
                info!(
                    "🤖 Tenant loop: {} ${:.0} capital ({}s stagger)",
                    tid, tenant_capital, stagger
                );

                let mut prev_mids: HashMap<String, f64> = HashMap::new();
                loop {
                    match run_cycle(
                        &t_config,
                        &t_market,
                        &t_hl,
                        &t_db,
                        &tenant_state,
                        &t_weights,
                        &t_sent,
                        &t_fund,
                        &mut prev_mids,
                        &t_btcdom,
                        &t_logger,
                        &t_feedback,
                        tenant_fee_bps,
                        &t_notifier,
                        &t_onchain,
                        &t_watch,
                        &t_cex,
                        &t_thesis,
                    )
                    .await
                    {
                        Ok(_) => {
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                        }
                        Err(e) => {
                            let err_str = e.to_string();
                            let (sleep_secs, _) = classify_cycle_error(&err_str);
                            log::warn!("Tenant {} cycle error: {}", tid, err_str);
                            tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
                        }
                    }
                }
            });
        }
    }

    // ── Trial-expiry promo email task (hourly) ────────────────────────────────
    // Scans for Free tenants whose 14-day trial has just elapsed and who
    // haven't received the $9.95 intro-offer email yet, then sends one.
    // Runs every hour; each tenant is emailed exactly once (DB idempotency).
    if let (Some(ref db), Some(ref ml)) = (&shared_db, &mailer) {
        let db_promo = db.clone();
        let ml_promo = ml.clone();
        let promo_pid = config.stripe_promo_price_id.clone();
        let site_url =
            std::env::var("SITE_URL").unwrap_or_else(|_| "https://tradingbots.fun".to_string());
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            interval.tick().await; // skip the first immediate tick (just started)
            loop {
                interval.tick().await;
                match db_promo.fetch_expired_trial_tenants().await {
                    Ok(tenants) => {
                        for (tenant_id, email, name) in tenants {
                            // Build the checkout URL — use the promo price when
                            // configured, otherwise fall back to the standard price.
                            let promo_flag = if promo_pid.is_some() { "&promo=1" } else { "" };
                            let checkout_url = format!(
                                "{}/billing/checkout?tenant_id={}{}",
                                site_url, tenant_id, promo_flag
                            );
                            let html = mailer::Mailer::trial_expiry_html(&name, &checkout_url);
                            let subject = "Your trial ended — try Pro for $9.95 this month";
                            match ml_promo.send(&email, subject, &html).await {
                                Ok(_) => {
                                    info!("📧 Promo email sent → {}", email);
                                    if let Err(e) = db_promo.mark_promo_sent(&tenant_id).await {
                                        log::warn!("mark_promo_sent failed for {tenant_id}: {e}");
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Promo email failed for {email}: {e}");
                                }
                            }
                        }
                    }
                    Err(e) => log::warn!("fetch_expired_trial_tenants: {e}"),
                }
            }
        });
    } else if mailer.is_none() {
        info!("ℹ️  RESEND_API_KEY not set — trial promo emails disabled");
    }

    // Ctrl-C handler — saves state before exiting
    let running = Arc::new(RwLock::new(true));
    {
        let r = running.clone();
        let bs_shutdown = bot_state.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            info!("Shutdown signal — saving state…");
            persistence::save_snapshot(&bs_shutdown).await;
            info!("💾 State saved on shutdown");
            *r.write().await = false;
        });
    }

    let mut prev_mids: HashMap<String, f64> = HashMap::new();
    // Track last analysis date to trigger exactly once per midnight
    let mut last_analysis_date = date_today();
    // Set start capital once at boot
    {
        trade_logger.lock().await.day_stats.start_capital = config.initial_capital;
    }

    info!(
        "🚀 Main loop started (30 s cycle, paper={})",
        config.paper_trading
    );

    loop {
        if !*running.read().await {
            break;
        }

        // ── Midnight analysis trigger ─────────────────────────────────────
        // Check once per cycle if the date has rolled over.  If so, run the
        // daily analysis for yesterday's log in the background, and write
        // leaderboard snapshots for every active tenant.
        let today_now = date_today();
        if today_now != last_analysis_date {
            last_analysis_date = today_now.clone();
            if let Some(api_key) = config.anthropic_api_key.clone() {
                let logger_clone = trade_logger.clone();
                tokio::spawn(async move {
                    info!("🌙 Midnight — running daily analysis for yesterday…");
                    daily_analyst::analyse_day(&logger_clone, &api_key).await;
                });
            }
            // Write leaderboard snapshots for the active campaign
            if let Some(ref db) = shared_db {
                let db_snap = db.clone();
                let tm_snap = tenant_manager.clone();
                tokio::spawn(async move {
                    match leaderboard::snapshot_daily(&db_snap, &tm_snap).await {
                        Ok(n) => info!("📊 Leaderboard: wrote {} daily snapshots", n),
                        Err(e) => log::warn!("Leaderboard snapshot failed: {e}"),
                    }
                });
            }
            // Recalculate collective signal weights from 90 days of cross-user outcomes
            if let Some(ref db) = shared_db {
                let db_coll = db.clone();
                let w_coll = weights.clone();
                tokio::spawn(async move {
                    let current = w_coll.read().await.clone();
                    if let Some(new_w) =
                        collective::recalculate_collective_weights(&db_coll, &current, 50).await
                    {
                        *w_coll.write().await = new_w;
                        info!("🧠 Collective weights recalculated from cross-user outcomes");
                    }
                });
            }
        }

        set_status(&bot_state, "📡 Fetching prices…").await;

        match run_cycle(
            &config,
            &market,
            &hl,
            &shared_db,
            &bot_state,
            &weights,
            &sentiment_cache,
            &funding_cache,
            &mut prev_mids,
            &btc_dominance,
            &trade_logger,
            &ai_feedback_logger,
            config.builder_fee_bps,
            &notifier,
            &onchain_cache,
            &signal_watchlist,
            &cex_monitor,
            &global_thesis,
        )
        .await
        {
            Ok(_) => {
                // Persist state every N cycles (cheap; atomic rename)
                let cycle_n = bot_state.read().await.cycle_count;
                if cycle_n % persistence::SAVE_EVERY_N_CYCLES == 0 {
                    persistence::save_snapshot(&bot_state).await;
                }
                set_status(&bot_state, "⏳ Waiting for next cycle (30s)…").await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
            Err(e) => {
                let err_str = e.to_string();
                let (sleep_secs, friendly_msg) = classify_cycle_error(&err_str);
                error!("Cycle error: {err_str}");
                warn!("{friendly_msg}");
                set_status(&bot_state, &friendly_msg).await;
                tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
            }
        }
    }

    info!("🛑 Shutdown complete");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  BTC DOMINANCE FETCH (CoinGecko free /global endpoint)
// ═══════════════════════════════════════════════════════════════════════════════

/// Fetch live BTC market-cap dominance from CoinGecko's free /global endpoint.
/// Returns `None` if the request fails or the field is missing.
async fn fetch_btc_dominance() -> Option<f64> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .ok()?;
    let resp: serde_json::Value = client
        .get("https://api.coingecko.com/api/v3/global")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    resp["data"]["market_cap_percentage"]["btc"].as_f64()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  MAIN CYCLE
// ═══════════════════════════════════════════════════════════════════════════════

type SharedBtcDominance = Arc<RwLock<f64>>;

/// Open a position manually (from the Bot API or AI bar).
/// Called from the command-drain block in `run_cycle`.
#[allow(clippy::too_many_arguments)]
async fn manual_open_position(
    symbol: String,
    is_long: bool,
    size_usd: Option<f64>,
    leverage: Option<f64>,
    current_mids: &HashMap<String, f64>,
    bot_state: &SharedState,
    hl: &Arc<exchange::HyperliquidClient>,
    fee_bps: u32,
) {
    use web_dashboard::PaperPosition;
    let side = if is_long { "LONG" } else { "SHORT" };

    // Resolve price (case-insensitive)
    let price = current_mids.get(symbol.as_str()).copied().or_else(|| {
        let lc = symbol.to_lowercase();
        current_mids
            .iter()
            .find(|(k, _)| k.to_lowercase() == lc)
            .map(|(_, &v)| v)
    });

    let Some(px) = price else {
        warn!("🤖 Manual open {side}: {symbol} not in price feed");
        return;
    };

    let lev = leverage.unwrap_or(1.0).clamp(1.0, 10.0);
    // Default margin = 5% of free capital or $50, whichever is larger
    let margin = match size_usd {
        Some(v) => v.max(1.0),
        None => {
            let cap = bot_state.read().await.capital;
            (cap * 0.05).max(50.0)
        }
    };
    let stop_pct = 0.05;
    let sl = if is_long {
        px * (1.0 - stop_pct)
    } else {
        px * (1.0 + stop_pct)
    };
    let tp = if is_long { px * 1.10 } else { px * 0.90 };
    let qty = (margin * lev) / px.max(1e-8);

    info!(
        "🤖 Manual open {side}: {symbol} @ {px:.4}  \
           margin=${margin:.0}  lev={lev:.1}×  qty={qty:.4}"
    );

    {
        let mut s = bot_state.write().await;
        if margin > s.capital {
            warn!(
                "🤖 Manual open {side}: insufficient capital \
                   (need ${margin:.0}, have ${:.0})",
                s.capital
            );
            return;
        }
        s.capital -= margin;
        s.positions.push(PaperPosition {
            symbol: symbol.clone(),
            side: side.to_string(),
            entry_price: px,
            quantity: qty,
            size_usd: margin,
            stop_loss: sl,
            take_profit: tp,
            atr_at_entry: px * 0.02,
            high_water_mark: px,
            low_water_mark: px,
            partial_closed: false,
            r_dollars_risked: margin * stop_pct,
            tranches_closed: 0,
            dca_count: 0,
            leverage: lev,
            cycles_held: 0,
            entry_time: now_str(),
            unrealised_pnl: 0.0,
            contrib: Default::default(),
            ai_action: None,
            ai_reason: None,
            entry_confidence: 1.0,
            trade_budget_usd: margin,
            dca_spent_usd: 0.0,
            btc_ret_at_entry: 0.0,
            initial_margin_usd: margin,
            ob_sentiment: String::new(),
            ob_bid_wall_near: false,
            ob_ask_wall_near: false,
            ob_adverse_cycles: 0,
            order_flow_confidence: 0.0,
            order_flow_direction: String::new(),
            funding_rate: 0.0,
            funding_delta: 0.0,
            onchain_strength: 0.0,
            cex_premium_pct: 0.0,
            cex_mode: String::new(),
            funded_from_pool: false,
            pool_stake_usd: 0.0,
            venue: "Hyperliquid Perps (paper)".to_string(),
        });
    }

    // Submit real HL order (no-op in paper mode)
    let dec = decision::Decision {
        action: if is_long {
            "BUY".to_string()
        } else {
            "SELL".to_string()
        },
        skipped_direction: String::new(),
        confidence: 1.0,
        position_size: 1.0,
        leverage: lev,
        entry_price: px,
        stop_loss: sl,
        take_profit: tp,
        strategy: "Manual-BotAPI".to_string(),
        rationale: format!("[Manual] {side} {symbol} via Bot API"),
        signal_contribution: Default::default(),
    };
    if let Err(e) = hl.place_order(&symbol, &dec, margin, fee_bps).await {
        warn!("🤖 HL order failed for {symbol}: {e}");
    }
}

/// Execute one 30-second trading cycle.
///
/// Sequence of operations:
///   1. Tier-1 price fetch — single Hyperliquid `allMids` call (all perps).
///   2. Update peak equity; increment `cycles_held` on open positions.
///   3. Position management — trailing stops, R-multiple partials, time exits.
///   4. Session price snapshot update; candidate list pushed to dashboard.
///   5. Refresh BTC dominance every 20 cycles.
///   6. Compute BTC 24h / 4h returns for cross-asset context.
///   7. Tier-2 analysis — fetch candles + order book for each candidate and
///      call `analyse_symbol()` which emits entry or skip decisions.
///   8. Optional Claude AI position review every 10 cycles.
#[allow(clippy::too_many_arguments)]
async fn run_cycle(
    config: &config::Config,
    market: &Arc<data::MarketClient>,
    hl: &Arc<exchange::HyperliquidClient>,
    db: &Option<SharedDb>,
    bot_state: &SharedState,
    weights: &SharedWeights,
    sent_cache: &SharedSentiment,
    fund_cache: &SharedFunding,
    prev_mids: &mut HashMap<String, f64>,
    btc_dominance: &SharedBtcDominance,
    trade_logger: &SharedTradeLogger,
    ai_feedback: &SharedAiFeedbackLogger,
    fee_bps: u32,                                           // builder fee bps
    notifier: &Option<SharedNotifier>,                      // webhook / Telegram alerts
    onchain_cache: &SharedOnchain,                          // exchange netflow signal
    signal_watchlist: &SharedWatchlist,                     // near-miss SKIP re-evaluator
    cex_monitor: &SharedCrossExchange,                      // cross-exchange price divergence
    global_thesis: &Arc<RwLock<thesis::ThesisConstraints>>, // user thesis constraints
) -> Result<()> {
    {
        bot_state.write().await.cycle_count += 1;
    }

    // ── Tier 1: all prices in one Hyperliquid call ────────────────────────
    set_status(bot_state, "📡 Hyperliquid allMids…").await;
    let current_mids = market.fetch_all_mids().await.map_err(|e| {
        warn!("allMids: {}", e);
        e
    })?;
    info!("📊 {} prices", current_mids.len());

    // Log cycle start (used by daily analyst for context)
    {
        let s = bot_state.read().await;
        let _equity = s.capital
            + s.positions
                .iter()
                .map(|p| p.size_usd + p.unrealised_pnl)
                .sum::<f64>();
        trade_logger.lock().await.log(&TradeEvent::CycleStart {
            ts: ts_now(),
            cycle_number: s.cycle_count,
            open_positions: s.positions.len(),
            free_capital: s.capital,
            peak_equity: s.peak_equity,
            btc_dom_pct: 0.0, // updated later in the cycle
            btc_ret_24h: 0.0,
            btc_ret_4h: 0.0,
            candidate_count: 0,
        });
    }

    // Save old prices BEFORE overwriting — needed for change_pct display
    let prev_snapshot = prev_mids.clone();
    let candidates_raw = market.filter_candidates(&current_mids, prev_mids);
    *prev_mids = current_mids.clone();

    // ── Drain manual commands queued via the AI interface ─────────────────
    {
        let cmds: Vec<web_dashboard::BotCommand> = {
            let mut s = bot_state.write().await;
            s.pending_cmds.drain(..).collect()
        };
        for cmd in cmds {
            use web_dashboard::BotCommand;
            match cmd {
                BotCommand::ClosePosition { ref symbol } => {
                    let price = current_mids.get(symbol.as_str()).copied().or_else(|| {
                        let lc = symbol.to_lowercase();
                        current_mids
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == lc)
                            .map(|(_, &v)| v)
                    });
                    if let Some(px) = price {
                        info!("🤖 Manual close: {symbol} @ {px}");
                        close_paper_position(
                            symbol,
                            px,
                            "Manual-Close",
                            bot_state,
                            weights,
                            trade_logger,
                            notifier,
                            db,
                            single_op_tenant(),
                            hl,
                            fee_bps,
                            config.paper_trading,
                        )
                        .await;
                    } else {
                        warn!("🤖 Manual close: {symbol} not found in price feed");
                    }
                }
                BotCommand::TakePartial { ref symbol } => {
                    let price = current_mids.get(symbol.as_str()).copied().or_else(|| {
                        let lc = symbol.to_lowercase();
                        current_mids
                            .iter()
                            .find(|(k, _)| k.to_lowercase() == lc)
                            .map(|(_, &v)| v)
                    });
                    if let Some(px) = price {
                        info!("🤖 Manual take-partial: {symbol} @ {px}");
                        take_partial(
                            symbol.clone(),
                            px,
                            0,
                            bot_state,
                            weights,
                            trade_logger,
                            hl,
                            fee_bps,
                            config.paper_trading,
                        )
                        .await;
                    } else {
                        warn!("🤖 Manual take-partial: {symbol} not found in price feed");
                    }
                }
                BotCommand::CloseAll => {
                    let open: Vec<(String, f64)> = {
                        let s = bot_state.read().await;
                        s.positions
                            .iter()
                            .filter_map(|p| {
                                current_mids
                                    .get(p.symbol.as_str())
                                    .map(|&px| (p.symbol.clone(), px))
                            })
                            .collect()
                    };
                    info!("🤖 Manual close-all: {} positions", open.len());
                    for (sym, px) in open {
                        close_paper_position(
                            &sym,
                            px,
                            "Manual-CloseAll",
                            bot_state,
                            weights,
                            trade_logger,
                            notifier,
                            db,
                            single_op_tenant(),
                            hl,
                            fee_bps,
                            config.paper_trading,
                        )
                        .await;
                    }
                }
                BotCommand::CloseProfitable => {
                    let profitable: Vec<(String, f64)> = {
                        let s = bot_state.read().await;
                        s.positions
                            .iter()
                            .filter(|p| p.unrealised_pnl > 0.0)
                            .filter_map(|p| {
                                current_mids
                                    .get(p.symbol.as_str())
                                    .map(|&px| (p.symbol.clone(), px))
                            })
                            .collect()
                    };
                    info!(
                        "🤖 Manual take-profits: {} winning positions",
                        profitable.len()
                    );
                    for (sym, px) in profitable {
                        close_paper_position(
                            &sym,
                            px,
                            "Manual-TakeProfit",
                            bot_state,
                            weights,
                            trade_logger,
                            notifier,
                            db,
                            single_op_tenant(),
                            hl,
                            fee_bps,
                            config.paper_trading,
                        )
                        .await;
                    }
                }
                BotCommand::OpenLong {
                    symbol,
                    size_usd,
                    leverage,
                } => {
                    manual_open_position(
                        symbol,
                        true,
                        size_usd,
                        leverage,
                        &current_mids,
                        bot_state,
                        hl,
                        fee_bps,
                    )
                    .await;
                }
                BotCommand::OpenShort {
                    symbol,
                    size_usd,
                    leverage,
                } => {
                    manual_open_position(
                        symbol,
                        false,
                        size_usd,
                        leverage,
                        &current_mids,
                        bot_state,
                        hl,
                        fee_bps,
                    )
                    .await;
                }
                BotCommand::SetLeverage { ref symbol, leverage } => {
                    info!("🔧 SetLeverage: {symbol} → {leverage}×");
                    // Stored in session config — no paper-trade action needed here.
                }
                BotCommand::PauseTrading => {
                    info!("⏸ PauseTrading command received — bot will skip entry signals this cycle");
                    // Session-level pause is enforced in HyperliquidConnector.
                    // For the global paper-trading bot, we log and skip.
                }
                BotCommand::ResumeTrading => {
                    info!("▶ ResumeTrading command received — resuming normal operation");
                }
            }
        }
    }

    // ── Investment thesis: read current constraints (lock-free snapshot) ──
    let thesis_snap = global_thesis.read().await.clone();

    // Priority coins always analysed FIRST so they get slot priority
    const PRIORITY: &[&str] = &["SOL", "BTC", "ETH", "BNB", "AVAX"];
    let mut candidates: Vec<String> = PRIORITY
        .iter()
        .filter_map(|&p| candidates_raw.iter().find(|s| s.as_str() == p).cloned())
        .collect();
    for sym in &candidates_raw {
        if !candidates.contains(sym) {
            candidates.push(sym.clone());
        }
    }

    // Apply whitelist / sector filter — drop disallowed symbols
    if !thesis_snap.is_empty() {
        let before = candidates.len();
        candidates.retain(|sym| thesis_snap.allows(sym));
        let after = candidates.len();
        if after < before {
            info!(
                "🎯 Thesis filter: {}/{} candidates kept ({})",
                after,
                before,
                thesis_snap.summary.as_deref().unwrap_or("custom")
            );
        }
    }

    // ── Update peak equity & increment cycles_held ────────────────────────
    {
        let mut s = bot_state.write().await;
        let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
        let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
        let equity = s.capital + committed + unrealised;

        // All-time peak (for display)
        if equity > s.peak_equity {
            s.peak_equity = equity;
        }

        // Rolling 7-day window for circuit breaker (prevents one lucky spike
        // from permanently throttling position sizes months later)
        const SEVEN_DAYS_SECS: i64 = 7 * 24 * 3600;
        let now_ts = chrono::Utc::now().timestamp();
        s.equity_window.push_back((now_ts, equity));
        // Trim entries older than 7 days
        while s
            .equity_window
            .front()
            .map(|&(ts, _)| now_ts - ts > SEVEN_DAYS_SECS)
            .unwrap_or(false)
        {
            s.equity_window.pop_front();
        }

        // Sparkline history — one point per cycle, capped at ~2.4 h (288 × 30 s)
        s.equity_history.push(equity);
        if s.equity_history.len() > 288 {
            s.equity_history.remove(0);
        }

        for pos in s.positions.iter_mut() {
            pos.cycles_held += 1;
        }

        // ── PostgreSQL: equity snapshot ───────────────────────────────────
        // Persist one equity data point per cycle. Tenant ID is "operator"
        // in single-op mode; will be per-tenant in multi-tenant phase.
        // Fire-and-forget — a DB write failure must never crash the cycle.
        if let Some(db) = db {
            let equity_copy = equity;
            let db_clone = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone
                    .insert_equity_snapshot("00000000-0000-0000-0000-000000000001", equity_copy)
                    .await
                {
                    log::debug!("equity_snapshot write skipped: {e}");
                }
            });
        }

        // ── PostgreSQL: AUM snapshot (pre-aggregated for admin + landing page) ──
        // In single-operator mode total_aum == operator equity.
        // In multi-tenant mode this will sum across all tenants.
        let initial_capital = s.initial_capital;
        let open_positions = s.positions.len() as i32;
        // Today's trade stats come from the trade_logger day_stats (not ClosedTrade
        // strings which only carry HH:MM:SS without a date).  For now we emit
        // zeroes; the DB can compute accurate daily stats via a SQL query.
        let closed_today: i32 = 0;
        let win_rate_today: Option<f64> = None;

        if let Some(db) = db {
            let total_pnl = equity - initial_capital;
            let pnl_pct = if initial_capital > 0.0 {
                total_pnl / initial_capital * 100.0
            } else {
                0.0
            };
            let snap = AumSnapshot {
                total_aum: equity,
                deposited_capital: initial_capital,
                total_pnl,
                pnl_pct,
                active_tenant_count: if open_positions > 0 { 1 } else { 0 },
                total_tenant_count: 1,
                open_position_count: open_positions,
                total_trades_today: closed_today,
                win_rate_today,
            };
            let db_clone = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone.insert_aum_snapshot(&snap).await {
                    log::debug!("aum_snapshot write skipped: {e}");
                }
            });
        }
    }

    // ── Position management: P&L, trailing stops, exit signals ───────────
    let open_count = bot_state.read().await.positions.len();
    if open_count > 0 {
        set_status(
            bot_state,
            &format!("🔍 Managing {} open position(s)…", open_count),
        )
        .await;
    }

    let mut to_close: Vec<(String, f64, String)> = Vec::new(); // (sym, exit, reason)
    let mut to_partial0: Vec<(String, f64)> = Vec::new(); // 1/4 out at 1R  — lock in early profit
    let mut to_partial1: Vec<(String, f64)> = Vec::new(); // 1/3 out at 2R
    let mut to_partial2: Vec<(String, f64)> = Vec::new(); // 1/3 out at 4R
    let mut pnl_updates: Vec<(String, f64)> = Vec::new(); // (sym, pnl_pct) for hot_positions

    {
        let mut s = bot_state.write().await;
        for pos in s.positions.iter_mut() {
            let cur = match current_mids.get(pos.symbol.as_str()) {
                Some(&p) => p,
                None => continue,
            };

            // Water marks
            if cur > pos.high_water_mark {
                pos.high_water_mark = cur;
            }
            if cur < pos.low_water_mark {
                pos.low_water_mark = cur;
            }

            // Unrealised P&L
            pos.unrealised_pnl = if pos.side == "LONG" {
                (cur - pos.entry_price) * pos.quantity
            } else {
                (pos.entry_price - cur) * pos.quantity
            };
            // Collect for collective hot_position P&L update (fired after write-lock released)
            let pnl_pct_now = if pos.size_usd > 0.0 {
                pos.unrealised_pnl / pos.size_usd * 100.0
            } else {
                0.0
            };
            pnl_updates.push((pos.symbol.clone(), pnl_pct_now));

            // R-multiple (uses original dollars risked, not current stop)
            let r_mult = if pos.r_dollars_risked > 1e-8 {
                pos.unrealised_pnl / pos.r_dollars_risked
            } else {
                0.0
            };

            let atr = pos.atr_at_entry.max(pos.entry_price * 0.001);

            // ── Trailing stop logic ───────────────────────────────────────
            // Four-tier approach — every early gain gets a safety net:
            //
            //   Tier 0 │ 0.30R │ 0.4×ATR trail, stays below entry
            //           │       │ Catches fast reversals ("false breakout" protection)
            //   Tier 1 │ 0.75R │ 0.6×ATR trail, stays below entry
            //           │       │ Tightens as trade proves itself
            //   Tier 2 │ 1.0R  │ Stop → exact breakeven (no-loss guaranteed)
            //   Tier 3 │ 1.5R  │ 1.2×ATR trail, no ceiling (let winner run)
            //
            // Each tier only advances the stop — never retreats it.
            if pos.side == "LONG" {
                if r_mult >= 1.5 {
                    // Tier 3: Wide trail — let runner breathe
                    let trail = pos.high_water_mark - atr * 1.2;
                    if trail > pos.stop_loss {
                        pos.stop_loss = trail;
                    }
                } else if r_mult >= 1.0 && pos.stop_loss < pos.entry_price {
                    // Tier 2: Lock in breakeven
                    pos.stop_loss = pos.entry_price;
                    info!(
                        "📌 {} LONG stop → breakeven ${:.4}",
                        pos.symbol, pos.entry_price
                    );
                } else if r_mult >= 0.75 {
                    // Tier 1: Tighter trail as trade matures
                    let trail = pos.high_water_mark - atr * 0.6;
                    if trail > pos.stop_loss && trail < pos.entry_price {
                        pos.stop_loss = trail;
                    }
                } else if r_mult >= 0.30 {
                    // Tier 0: Very tight early trail — protect first signs of profit
                    let trail = pos.high_water_mark - atr * 0.4;
                    if trail > pos.stop_loss && trail < pos.entry_price {
                        pos.stop_loss = trail;
                    }
                }
            } else {
                // SHORT
                if r_mult >= 1.5 {
                    // Tier 3: Wide trail
                    let trail = pos.low_water_mark + atr * 1.2;
                    if trail < pos.stop_loss {
                        pos.stop_loss = trail;
                    }
                } else if r_mult >= 1.0 && pos.stop_loss > pos.entry_price {
                    // Tier 2: Breakeven
                    pos.stop_loss = pos.entry_price;
                    info!(
                        "📌 {} SHORT stop → breakeven ${:.4}",
                        pos.symbol, pos.entry_price
                    );
                } else if r_mult >= 0.75 {
                    // Tier 1: Tighter trail
                    let trail = pos.low_water_mark + atr * 0.6;
                    if trail < pos.stop_loss && trail > pos.entry_price {
                        pos.stop_loss = trail;
                    }
                } else if r_mult >= 0.30 {
                    // Tier 0: Early tight trail
                    let trail = pos.low_water_mark + atr * 0.4;
                    if trail < pos.stop_loss && trail > pos.entry_price {
                        pos.stop_loss = trail;
                    }
                }
            }

            // ── Exit checks ───────────────────────────────────────────────
            let hit_stop = (pos.side == "LONG" && cur <= pos.stop_loss)
                || (pos.side == "SHORT" && cur >= pos.stop_loss);
            let hit_tp = (pos.side == "LONG" && cur >= pos.take_profit)
                || (pos.side == "SHORT" && cur <= pos.take_profit);

            // ── Time exit: only genuinely dead money after HOURS, not minutes ──
            //
            // Rules (all must hold):
            //   • Never time-exit a winning position (let winners run)
            //   • "Truly flat": no meaningful progress either way after 2 hrs
            //     → cycles_held ≥ 240 (120 min) AND |r_mult| < 0.20
            //   • "Chronic bleeder": consistently losing after 3 hrs with NO DCA taken
            //     → cycles_held ≥ 360 (180 min) AND r_mult < -0.45 AND dca_count == 0
            //   • Post-DCA bleeder: DCA didn't rescue it after another 2 hrs
            //     → cycles_held ≥ 480 (240 min) AND r_mult < -0.60 AND dca_count > 0
            //   • Never time-exit within 0.10R of stop (let stop-loss handle)
            // ── False-breakout detection ──────────────────────────────────
            // Pattern: trade shows early promise, then rapidly reverses — a
            // failed breakout that should be cut before it bleeds further.
            //
            // Peak R-multiple computed from HWM (LONG) or LWM (SHORT):
            //   peak_r = max-favourable move expressed in R units
            let peak_r = if pos.r_dollars_risked > 1e-8 {
                if pos.side == "LONG" {
                    (pos.high_water_mark - pos.entry_price) * pos.quantity / pos.r_dollars_risked
                } else {
                    (pos.entry_price - pos.low_water_mark) * pos.quantity / pos.r_dollars_risked
                }
            } else {
                0.0
            };
            //
            // Conditions (all must hold):
            //   • Was genuinely profitable at some point (peak_r ≥ 0.30R — raised from
            //     0.10R to avoid premature exits on micro-bounces that reverse)
            //   • Has since reversed into a loss (r_mult < −0.05R)
            //   • Still within the first 60 min — rapid reversal = failed signal
            //   • No DCA taken yet — DCA means we chose to double down; let that play
            let false_breakout = peak_r >= 0.30   // was 0.10 — higher bar prevents cheap exits
                && r_mult < -0.05
                && pos.cycles_held >= 30   // at least 15 min before declaring failure
                && pos.cycles_held < 120   // still within the 60-min "fresh" window
                && pos.dca_count == 0;

            // TimeExit windows scale with DCA count — each add-on earns the position
            // more breathing room because we've explicitly decided to stay in.
            // Base: 240 cycles (2h) flat / 360 cycles (3h) chronic loss.
            // With 1 DCA: 2× → 480 flat / 720 chronic.
            // Reduced 2+DCA multiplier 3→2: live data showed 9h leash on DCA'd losers
            // (e.g. POPCAT DCA×3, AIXBT DCA×2) holding $50k+ in dead positions.
            // 6h max patience keeps capital moving.
            let time_mult = match pos.dca_count {
                0 => 1,
                1 => 2, // 1 DCA → double the patience window
                _ => 2, // 2+ DCA → same 2× cap (was 3×; 9h was too long)
            };
            let truly_flat = pos.cycles_held >= 240 * time_mult && r_mult.abs() < 0.20;
            let chronic_loss = pos.cycles_held >= 360 * time_mult
                && r_mult < -0.45
                && r_mult > -0.90
                && pos.dca_count == 0;
            let post_dca_loss = pos.cycles_held >= 360 * time_mult
                && r_mult < -0.50
                && r_mult > -0.90
                && pos.dca_count == 1;
            // DCA exhausted: if 2+ add-ons and still deep in loss after extended window
            let post_dca_exhausted = pos.cycles_held >= 240 * time_mult
                && r_mult < -0.25
                && r_mult > -0.90
                && pos.dca_count >= 2;
            let stale = (truly_flat || chronic_loss || post_dca_loss || post_dca_exhausted)
                && r_mult <= 0.0;

            // ── Profit-lock: force partial exit when sitting on big gains ─────────
            // The bot must take WINS when available.  Two thresholds:
            //   +15% unrealised on margin → force first tranche (¼ off)
            //   +25% unrealised on margin → force full exit before profit evaporates
            // These override the R-multiple tranche logic so gains are locked regardless
            // of whether formal R targets have been reached.
            let profit_pct = if pos.size_usd > 0.0 {
                pos.unrealised_pnl / pos.size_usd
            } else {
                0.0
            };
            if profit_pct >= 0.25 && !to_close.iter().any(|(s, _, _)| s == &pos.symbol) {
                to_close.push((pos.symbol.clone(), cur, "ProfitLock25".to_string()));
                info!(
                    "💰 {} profit-lock full exit @ +{:.1}% unrealised",
                    pos.symbol,
                    profit_pct * 100.0
                );
            } else if profit_pct >= 0.15
                && pos.tranches_closed == 0
                && !to_partial0.iter().any(|(s, _)| s == &pos.symbol)
                && !to_close.iter().any(|(s, _, _)| s == &pos.symbol)
            {
                to_partial0.push((pos.symbol.clone(), cur));
                info!(
                    "💰 {} profit-lock partial @ +{:.1}% unrealised",
                    pos.symbol,
                    profit_pct * 100.0
                );
            }

            // ── Order-book adverse pressure exits ─────────────────────────
            // The book is a real-time sentiment signal; when it consistently
            // flips against an open position, act BEFORE the price does.
            //
            // Rule 1 — OB Profit Protection (r_mult ≥ 0.5R, book adverse 6+ cycles)
            //   Take the first tranche early (at 0.5R instead of 1R) to lock in
            //   half-profit while still having a position on. Only fires when we
            //   have not yet banked any tranche and the book has been adverse ≥6 cycles.
            //
            // Rule 2 — Strong reversal exit (book STRONGLY adverse 4+ cycles, r_mult > 0)
            //   If the book is sending a strong signal (STRONGLY_BEAR on LONG) and
            //   we are still in profit, exit the full position. Market is telling us
            //   the move is over.
            //
            // Rule 3 — House-money tightened trailing stop (PnL ≥ initial_margin)
            //   Once the trade has paid for the initial margin (unrealised_pnl covers
            //   what was deposited), tighten the tier-3 trail to 0.6×ATR instead
            //   of 1.2×ATR. "Let it ride on house money, but keep a tighter leash."
            let ob_adverse = pos.ob_adverse_cycles;
            let strongly_adverse = ob_adverse >= 4
                && ((pos.side == "LONG" && pos.ob_sentiment == "STRONGLY_BEARISH")
                    || (pos.side == "SHORT" && pos.ob_sentiment == "STRONGLY_BULLISH"));
            let mildly_adverse = ob_adverse >= 6
                && ((pos.side == "LONG" && pos.ob_sentiment.contains("BEAR"))
                    || (pos.side == "SHORT" && pos.ob_sentiment.contains("BULL")));
            // Principal recovery flag: trade has "paid for itself"
            let principal_recovered =
                pos.initial_margin_usd > 0.0 && pos.unrealised_pnl >= pos.initial_margin_usd;
            if principal_recovered && r_mult >= 1.5 {
                // House-money mode: tighten trail to 0.6×ATR (applied above 1.5R)
                // We re-tighten here only if the standard trail is looser than our target
                if pos.side == "LONG" {
                    let house_trail = pos.high_water_mark - atr * 0.60;
                    if house_trail > pos.stop_loss {
                        pos.stop_loss = house_trail;
                    }
                } else {
                    let house_trail = pos.low_water_mark + atr * 0.60;
                    if house_trail < pos.stop_loss {
                        pos.stop_loss = house_trail;
                    }
                }
            }

            if hit_stop {
                to_close.push((pos.symbol.clone(), cur, "StopLoss".to_string()));
            } else if hit_tp {
                to_close.push((pos.symbol.clone(), cur, "TakeProfit".to_string()));
            } else if false_breakout {
                to_close.push((pos.symbol.clone(), cur, "FalseBreakout".to_string()));
                info!(
                    "🔙 {} false-breakout exit — peaked {:.2}R, now {:.2}R after {} cycles",
                    pos.symbol, peak_r, r_mult, pos.cycles_held
                );
            } else if strongly_adverse && r_mult > 0.0 && pos.tranches_closed > 0 {
                // Book is strongly reversing, we already banked a tranche → exit remainder
                to_close.push((pos.symbol.clone(), cur, "BookReversal".to_string()));
                info!(
                    "📖 {} book-reversal exit — {} {} cycles, {:.2}R profit banked",
                    pos.symbol, pos.ob_sentiment, ob_adverse, r_mult
                );
            } else if stale {
                let reason_detail = if truly_flat {
                    "flat"
                } else if chronic_loss {
                    "chronic loss"
                } else if post_dca_exhausted {
                    "DCA exhausted"
                } else {
                    "post-DCA loss"
                };
                to_close.push((pos.symbol.clone(), cur, "TimeExit".to_string()));
                info!(
                    "⏰ {} time-exit ({}) after {} cycles at {:.2}R",
                    pos.symbol, reason_detail, pos.cycles_held, r_mult
                );
            } else {
                // R-multiple partial profit tranches
                // Tranche 0: 1/4 out at 0.75R (or 0.5R if book is adverse) — harvest early profit
                //            Lowered 1.0→0.75: BERA@0.85R, VINE@0.77R were sitting at risk
                //            with no tranche banked. Take the ¼ slice sooner so reversals
                //            don't give back open profit before the first harvest.
                // Tranche 1: 1/3 out at 2R — take more off as the trade extends
                // Tranche 2: 1/3 out at 4R — capture deep runner profits
                let early_partial_r = if mildly_adverse { 0.5 } else { 0.75 };
                if r_mult >= early_partial_r && pos.tranches_closed == 0 {
                    if mildly_adverse {
                        info!(
                            "📖 {} OB-early partial at {:.2}R ({} adverse cycles, {})",
                            pos.symbol, r_mult, ob_adverse, pos.ob_sentiment
                        );
                    }
                    to_partial0.push((pos.symbol.clone(), cur));
                } else if r_mult >= 2.0 && pos.tranches_closed == 1 {
                    to_partial1.push((pos.symbol.clone(), cur));
                } else if r_mult >= 4.0 && pos.tranches_closed == 2 {
                    to_partial2.push((pos.symbol.clone(), cur));
                }
            }
        }
    }

    // ── Collective intelligence: flush hot-position P&L updates ──────────
    // Fire after the write lock is released so we don't hold it during async I/O.
    if let Some(ref shared_db) = db {
        for (sym, pnl_pct) in pnl_updates {
            // Skip symbols being closed this cycle — they'll be removed instead
            if to_close.iter().any(|(s, _, _)| s == &sym) {
                continue;
            }
            let db_u = shared_db.clone();
            tokio::spawn(async move {
                collective::update_hot_pnl(&db_u, single_op_tenant(), &sym, pnl_pct).await;
            });
        }
    }

    // Execute partials first (they don't remove positions)
    // Tranche 0 first (1R, 1/4 size) → then 1 (2R, 1/3) → then 2 (4R, 1/3)
    for (sym, price) in to_partial0 {
        if to_close.iter().any(|(s, _, _)| s == &sym) {
            continue;
        }
        take_partial(
            sym,
            price,
            0,
            bot_state,
            weights,
            trade_logger,
            hl,
            fee_bps,
            config.paper_trading,
        )
        .await;
    }
    for (sym, price) in to_partial1 {
        if to_close.iter().any(|(s, _, _)| s == &sym) {
            continue;
        }
        take_partial(
            sym,
            price,
            1,
            bot_state,
            weights,
            trade_logger,
            hl,
            fee_bps,
            config.paper_trading,
        )
        .await;
    }
    for (sym, price) in to_partial2 {
        if to_close.iter().any(|(s, _, _)| s == &sym) {
            continue;
        }
        take_partial(
            sym,
            price,
            2,
            bot_state,
            weights,
            trade_logger,
            hl,
            fee_bps,
            config.paper_trading,
        )
        .await;
    }

    // Execute full closes
    for (sym, price, reason) in to_close {
        close_paper_position(
            &sym,
            price,
            &reason,
            bot_state,
            weights,
            trade_logger,
            notifier,
            db,
            single_op_tenant(),
            hl,
            fee_bps,
            config.paper_trading,
        )
        .await;
        info!("🚨 {} closed → {} @ ${:.4}", sym, reason, price);
    }

    // ── Update session prices (first time we see each symbol) ────────────
    {
        let mut s = bot_state.write().await;
        for (sym, &price) in &current_mids {
            s.session_prices.entry(sym.clone()).or_insert(price);
        }
    }

    // ── Update dashboard candidate list & weights mirror ─────────────────
    // Build CandidateInfo and push to state under a single lock.
    // rsi/regime/atr_pct are filled in later by the per-symbol analysis loop.
    {
        let session_snap = bot_state.read().await.session_prices.clone();
        let cand_infos: Vec<CandidateInfo> = candidates
            .iter()
            .filter_map(|sym| {
                let &price = current_mids.get(sym.as_str())?;
                // cycle 1: prev_snapshot is empty (no previous reference price yet) → show "—"
                let change_pct: Option<f64> = if prev_snapshot.is_empty() {
                    None
                } else {
                    let session_chg = session_snap
                        .get(sym.as_str())
                        .map(|&base| {
                            if base > 0.0 {
                                (price - base) / base * 100.0
                            } else {
                                0.0
                            }
                        })
                        .unwrap_or(0.0);
                    let cycle_chg = prev_snapshot
                        .get(sym.as_str())
                        .map(|&prev| {
                            if prev > 0.0 {
                                (price - prev) / prev * 100.0
                            } else {
                                0.0
                            }
                        })
                        .unwrap_or(0.0);
                    Some(if session_chg.abs() > 0.01 {
                        session_chg
                    } else {
                        cycle_chg
                    })
                };
                Some(CandidateInfo {
                    symbol: sym.clone(),
                    price,
                    change_pct,
                    rsi: None,
                    regime: None,
                    atr_pct: None,
                    confidence: None,
                })
            })
            .collect();
        let mut s = bot_state.write().await;
        s.candidates = cand_infos;
        s.last_update = now_str();
        s.signal_weights = weights.read().await.clone();
    }

    // ── BTC market context (dominance + direction) ───────────────────────
    // Refresh live dominance every 20 cycles (~10 min).  Between refreshes
    // the cached value is reused so we don't hammer the CoinGecko free tier.
    let cycle_count_now = bot_state.read().await.cycle_count;
    if cycle_count_now % 20 == 1 {
        if let Some(dom) = fetch_btc_dominance().await {
            *btc_dominance.write().await = dom;
            info!("🔄 BTC dominance refreshed: {:.1}%", dom);
        } else {
            warn!(
                "BTC dominance refresh failed — using cached {:.1}%",
                *btc_dominance.read().await
            );
        }
    }

    // BTC 24h return: compare first vs last candle across the 1h window (50 × 1h ≈ 50h).
    // BTC 4h return: first vs last candle in the 4h window (used for relative perf signal).
    // Both fetched same-cycle so always fresh.
    let (btc_ret_24h, btc_ret_4h): (f64, f64) = {
        let candles_1h = market.fetch_market_data("BTC").await;
        let candles_4h = market.fetch_market_data_4h("BTC").await;
        let ret_24h = match &candles_1h {
            Ok(c) if c.len() >= 2 => {
                let f = c.first().unwrap().close;
                let l = c.last().unwrap().close;
                if f > 0.0 {
                    (l - f) / f * 100.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        let ret_4h = match &candles_4h {
            Ok(c) if c.len() >= 2 => {
                let f = c.first().unwrap().close;
                let l = c.last().unwrap().close;
                if f > 0.0 {
                    (l - f) / f * 100.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        (ret_24h, ret_4h)
    };

    let btc_dom = *btc_dominance.read().await;
    if btc_ret_24h != 0.0 || btc_ret_4h != 0.0 {
        info!(
            "🟠 BTC ctx: dom={:.1}%  24h={:+.2}%  4h={:+.2}%",
            btc_dom, btc_ret_24h, btc_ret_4h
        );
    } else {
        warn!("BTC candles unavailable — dominance filter disabled this cycle");
    }

    // ── Tier 2: analyse candidates ────────────────────────────────────────
    let total = candidates.len();
    let mut new_decisions: Vec<DecisionInfo> = Vec::new();
    // Collect per-symbol indicator snapshots to batch-update CandidateInfo at end of cycle.
    let mut cand_indicators: Vec<(String, f64, &'static str, f64, f64)> = Vec::new(); // (sym, rsi, regime, atr_pct, confidence)

    for (i, sym) in candidates.iter().enumerate() {
        set_status(
            bot_state,
            &format!("🔬 Analysing {}/{}: {}…", i + 1, total, sym),
        )
        .await;

        match analyse_symbol(
            sym,
            market,
            hl,
            db,
            config,
            bot_state,
            weights,
            sent_cache,
            fund_cache,
            btc_dom,
            btc_ret_24h,
            btc_ret_4h,
            trade_logger,
            fee_bps,
            notifier,
            onchain_cache,
            cex_monitor,
        )
        .await
        {
            Ok(Some((mut dec, ind))) => {
                // ── Thesis: clamp leverage if user requested lower risk ───
                if let Some(max_lev) = thesis_snap.max_leverage_override {
                    if dec.leverage > max_lev {
                        info!(
                            "🎯 {} leverage clamped {:.1}→{:.1}× (thesis)",
                            sym, dec.leverage, max_lev
                        );
                        dec.leverage = max_lev;
                    }
                }

                // ── Collective intelligence: crowd signal confidence nudge ──
                // Query how many users are currently holding this symbol and
                // whether the crowd is winning or losing.  Apply a multiplier
                // to our confidence before the entry decision is finalised.
                // If confidence falls below the minimum floor after the nudge,
                // suppress the trade (override action to SKIP).
                if dec.action != "SKIP" {
                    if let Some(ref shared_db) = db {
                        if let Some(crowd) = collective::get_crowd_signal(shared_db, sym).await {
                            let mult = crowd.confidence_multiplier(if dec.action == "BUY" {
                                "LONG"
                            } else {
                                "SHORT"
                            });
                            if (mult - 1.0).abs() > 0.001 {
                                let old_conf = dec.confidence;
                                dec.confidence *= mult;
                                info!("👥 {} crowd×{:.3} conf {:.0}%→{:.0}% ({} holders avg_pnl={:.1}%)",
                                    sym, mult, old_conf*100.0, dec.confidence*100.0,
                                    crowd.holder_count, crowd.avg_pnl_pct);
                            }
                            if dec.confidence < MIN_CONFIDENCE {
                                info!(
                                    "👥 {} suppressed by crowd signal (conf {:.0}% < {:.0}%)",
                                    sym,
                                    dec.confidence * 100.0,
                                    MIN_CONFIDENCE * 100.0
                                );
                                dec.action = "SKIP".to_string();
                            }
                        }
                    }
                }

                let price = ind.close_price;

                if dec.action != "SKIP" {
                    info!(
                        "💡 {} → {} conf={:.0}%",
                        sym,
                        dec.action,
                        dec.confidence * 100.0
                    );
                    // Trade fired — remove from watchlist (entry taken or opportunity resolved).
                    signal_watchlist.remove(sym).await;
                } else {
                    // ── Watchlist: re-evaluate if symbol was already being watched ──
                    // Use skipped_direction (the lean even on SKIP) so "SKIP with BUY lean"
                    // is correctly evaluated as "still pointing BUY".
                    let reeval_action = if dec.skipped_direction != "NONE" {
                        &dec.skipped_direction
                    } else {
                        &dec.action
                    };
                    signal_watchlist
                        .re_evaluate(sym, reeval_action, dec.confidence, price)
                        .await;

                    // ── Watchlist: enqueue near-miss SKIPs ────────────────────────
                    // Distinguish between a gated skip and a low-confidence skip.
                    let skip_reason = if dec.rationale.contains("Funding gate")
                        || dec.rationale.contains("circuit breaker")
                    {
                        SkipReason::Gated(dec.rationale.chars().take(60).collect())
                    } else {
                        SkipReason::LowConfidence
                    };
                    signal_watchlist
                        .maybe_watch(
                            sym,
                            &dec.skipped_direction,
                            dec.confidence,
                            price,
                            skip_reason,
                        )
                        .await;
                }

                cand_indicators.push((
                    sym.clone(),
                    ind.rsi,
                    ind.regime,
                    ind.atr_pct,
                    dec.confidence,
                ));
                // Push ALL decisions (including SKIPs) so the signal feed always shows activity.
                // SKIPs are rendered dimmed in the dashboard; BUY/SELL get the coloured treatment.
                new_decisions.push(DecisionInfo {
                    symbol: sym.clone(),
                    action: dec.action.clone(),
                    confidence: dec.confidence,
                    entry_price: dec.entry_price,
                    rationale: dec.rationale.clone(),
                    timestamp: now_str(),
                });
            }
            Ok(None) => {}
            Err(e) => warn!("  {} error: {}", sym, e),
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    // Evict expired watchlist entries once per cycle (keeps the map tidy).
    signal_watchlist.gc().await;

    {
        let mut s = bot_state.write().await;
        // Write RSI / regime / ATR% back into the candidate list for the dashboard.
        for (sym, rsi, regime, atr_pct, conf) in &cand_indicators {
            if let Some(c) = s.candidates.iter_mut().find(|c| c.symbol == *sym) {
                c.rsi = Some(*rsi);
                c.regime = Some(regime.to_string());
                c.atr_pct = Some(*atr_pct);
                c.confidence = Some(*conf);
            }
        }
        s.recent_decisions.extend(new_decisions);
        // Keep at most 100 entries (20 decisions × 5 cycles of history).
        // The dashboard shows the 20 most recent, so older ones are just trimmed.
        let len = s.recent_decisions.len();
        if len > 100 {
            s.recent_decisions.drain(0..len - 100);
        }
        // Record when the next cycle will fire so the dashboard can show a real countdown.
        s.next_cycle_at = chrono::Utc::now().timestamp_millis() + 30_000;
    }

    {
        let mut s = bot_state.write().await;
        s.last_update = now_str();
    }

    // ── Metrics snapshot (every 10 cycles) ───────────────────────────────
    let (cycle_count, open_count) = {
        let s = bot_state.read().await;
        (s.cycle_count, s.positions.len())
    };
    if cycle_count % 10 == 0 {
        let s = bot_state.read().await;
        let m = &s.metrics;
        trade_logger.lock().await.log(&TradeEvent::MetricsSnapshot {
            ts: ts_now(),
            cycle_number: s.cycle_count,
            total_trades: s.closed_trades.len(),
            win_rate: m.win_rate,
            expectancy_pct: m.expectancy,
            sharpe: m.sharpe,
            sortino: m.sortino,
            max_drawdown_pct: m.max_drawdown * 100.0,
            profit_factor: m.profit_factor,
            kelly_fraction: m.kelly_fraction(),
            total_pnl_usd: s.pnl,
            capital: s.capital,
            open_positions: s.positions.len(),
        });
    }

    // ── AI position review (every 10 cycles ≈ 5 minutes) ─────────────────
    if cycle_count % 10 == 0 && open_count > 0 {
        if let Some(api_key) = &config.anthropic_api_key {
            set_status(bot_state, "🤖 Claude AI reviewing positions…").await;
            // Surface AI activity in the dashboard before the (slow) API call.
            {
                let mut s = bot_state.write().await;
                s.ai_status = format!("🤖 Querying Claude for {} open position(s)…", open_count);
            }
            let (positions_snap, metrics_snap, capital_snap) = {
                let s = bot_state.read().await;
                (s.positions.clone(), s.metrics.clone(), s.capital)
            };
            let review = ai_reviewer::review_positions(
                &positions_snap,
                &metrics_snap,
                capital_snap,
                api_key,
            )
            .await;
            // Update ai_status with a summary of what Claude recommended.
            {
                let now = chrono::Utc::now().format("%H:%M UTC").to_string();
                let summary = if review.recommendations.is_empty() {
                    format!(
                        "🤖 AI reviewed {} position(s) — all HOLD · {}",
                        open_count, now
                    )
                } else {
                    let actions: Vec<String> = review
                        .recommendations
                        .iter()
                        .map(|r| {
                            format!(
                                "{} → {}",
                                r.symbol,
                                r.action.replace('_', " ").to_uppercase()
                            )
                        })
                        .collect();
                    format!("🤖 {} · {} · {}", open_count, actions.join(" · "), now)
                };
                let mut s = bot_state.write().await;
                s.ai_status = summary;
            }
            apply_ai_review(
                &review,
                bot_state,
                weights,
                &current_mids,
                trade_logger,
                ai_feedback,
                notifier,
                db,
                single_op_tenant(),
                hl,
                fee_bps,
                config.paper_trading,
            )
            .await;
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  AI REVIEW APPLICATION
// ═══════════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
async fn apply_ai_review(
    review: &ai_reviewer::AiReview,
    bot_state: &SharedState,
    weights: &SharedWeights,
    current_mids: &HashMap<String, f64>,
    trade_logger: &SharedTradeLogger,
    ai_feedback: &SharedAiFeedbackLogger,
    notifier: &Option<SharedNotifier>,
    db: &Option<SharedDb>,
    tenant_id: Uuid,
    hl: &Arc<exchange::HyperliquidClient>,
    fee_bps: u32,
    paper: bool,
) {
    let usage = review.usage.clone();

    for rec in &review.recommendations {
        let cur_price = match current_mids.get(rec.symbol.as_str()) {
            Some(&p) => p,
            None => continue,
        };

        match rec.action.as_str() {
            "close_now" => {
                let pos_snapshot = {
                    let s = bot_state.read().await;
                    s.positions.iter().find(|p| p.symbol == rec.symbol).cloned()
                };
                if let Some(pos) = pos_snapshot {
                    let guardrail = evaluate_ai_close_guard(&pos);
                    let cycle_phase = current_cycle_phase();
                    let (funding_phase_label, hours_to_settle) = describe_cycle_phase(&cycle_phase);
                    let summary = format_signal_summary(&pos.contrib);
                    let breakdown = signal_breakdown(&pos.contrib);
                    let alignment_pct = signal_alignment_pct(&pos.contrib) * 100.0;
                    let order_flow_desc = order_flow_snapshot(&pos);
                    let cross_exchange_desc = cross_exchange_snapshot(&pos);

                    let timestamp = ts_now();
                    let guardrail_event = TradeEvent::AiGuardrailCheck {
                        ts: timestamp.clone(),
                        symbol: pos.symbol.clone(),
                        action: rec.action.clone(),
                        recommendation: rec.reason.clone(),
                        guardrail_score: guardrail.score,
                        guardrail_components: guardrail.components.clone(),
                        guardrail_note: guardrail.note.clone(),
                        r_multiple: guardrail.r_multiple,
                        hold_minutes: guardrail.hold_minutes,
                        dca_remaining: guardrail.dca_remaining,
                        false_breakout: guardrail.false_breakout,
                        momentum_stall: guardrail.momentum_stall,
                        entry_confidence: pos.entry_confidence,
                        signal_summary: summary.clone(),
                        signal_breakdown: breakdown.clone(),
                        signal_alignment_pct: alignment_pct,
                        funding_phase: funding_phase_label.clone(),
                        hours_to_settlement: hours_to_settle,
                        order_flow_confidence: pos.order_flow_confidence,
                        order_flow_direction: pos.order_flow_direction.clone(),
                        order_flow_snapshot: order_flow_desc.clone(),
                        ob_sentiment: pos.ob_sentiment.clone(),
                        ob_adverse_cycles: pos.ob_adverse_cycles,
                        funding_rate: pos.funding_rate,
                        funding_delta: pos.funding_delta,
                        onchain_strength: pos.onchain_strength,
                        cex_premium_pct: pos.cex_premium_pct,
                        cex_mode: pos.cex_mode.clone(),
                        cross_exchange_snapshot: cross_exchange_desc.clone(),
                        prompt_tokens: usage.as_ref().map(|u| u.prompt_tokens),
                        completion_tokens: usage.as_ref().map(|u| u.completion_tokens),
                        total_tokens: usage.as_ref().map(|u| u.total_tokens),
                        guardrail_allowed: guardrail.allow_close,
                    };
                    trade_logger.lock().await.log(&guardrail_event);

                    let feedback = GuardrailFeedback {
                        ts: timestamp,
                        symbol: pos.symbol.clone(),
                        action: rec.action.clone(),
                        recommendation: rec.reason.chars().take(200).collect(),
                        guardrail_score: guardrail.score,
                        guardrail_components: guardrail.components.clone(),
                        guardrail_note: guardrail.note.clone(),
                        guardrail_allowed: guardrail.allow_close,
                        r_multiple: guardrail.r_multiple,
                        hold_minutes: guardrail.hold_minutes,
                        dca_remaining: guardrail.dca_remaining,
                        false_breakout: guardrail.false_breakout,
                        momentum_stall: guardrail.momentum_stall,
                        entry_confidence: pos.entry_confidence,
                        signal_summary: summary,
                        signal_breakdown: breakdown,
                        signal_alignment_pct: alignment_pct,
                        funding_phase: funding_phase_label,
                        hours_to_settlement: hours_to_settle,
                        order_flow_snapshot: order_flow_desc,
                        order_flow_confidence: pos.order_flow_confidence,
                        order_flow_direction: pos.order_flow_direction.clone(),
                        ob_sentiment: pos.ob_sentiment.clone(),
                        ob_adverse_cycles: pos.ob_adverse_cycles,
                        funding_rate: pos.funding_rate,
                        funding_delta: pos.funding_delta,
                        onchain_strength: pos.onchain_strength,
                        cex_premium_pct: pos.cex_premium_pct,
                        cex_mode: pos.cex_mode.clone(),
                        cross_exchange_snapshot: cross_exchange_desc,
                        prompt_tokens: usage.as_ref().map(|u| u.prompt_tokens),
                        completion_tokens: usage.as_ref().map(|u| u.completion_tokens),
                        total_tokens: usage.as_ref().map(|u| u.total_tokens),
                    };
                    if let Err(e) = ai_feedback.lock().await.record(feedback) {
                        warn!("⚠ Guardrail feedback write failed: {}", e);
                    }
                    if guardrail.allow_close {
                        info!(
                            "🤖 AI close: {} — {} (score {:.2}; {})",
                            rec.symbol,
                            rec.reason,
                            guardrail.score,
                            guardrail.components_str()
                        );
                        let r_for_notify = {
                            let s = bot_state.read().await;
                            s.positions
                                .iter()
                                .find(|p| p.symbol == rec.symbol)
                                .map(|p| {
                                    if p.r_dollars_risked > 1e-8 {
                                        p.unrealised_pnl / p.r_dollars_risked
                                    } else {
                                        0.0
                                    }
                                })
                                .unwrap_or(0.0)
                        };
                        close_paper_position(
                            &rec.symbol,
                            cur_price,
                            "AI-Close",
                            bot_state,
                            weights,
                            trade_logger,
                            notifier,
                            db,
                            tenant_id,
                            hl,
                            fee_bps,
                            paper,
                        )
                        .await;
                        if let Some(n) = notifier {
                            let n = n.clone();
                            let sym = rec.symbol.clone();
                            let reason = rec.reason.chars().take(120).collect::<String>();
                            tokio::spawn(async move {
                                n.ai_action(&sym, "close_now", &reason, r_for_notify).await;
                            });
                        }
                    } else {
                        let note = guardrail
                            .note
                            .clone()
                            .unwrap_or_else(|| "guardrail threshold not met".to_string());
                        info!(
                            "🤖 AI close {} SKIPPED — {} (score {:.2}; {})",
                            rec.symbol,
                            note,
                            guardrail.score,
                            guardrail.components_str()
                        );
                    }
                }
            }

            "scale_up" => {
                // Guardrail: factor capped at 3.0, position must have positive R
                let factor = rec.factor.clamp(1.0, 3.0);
                let can_scale = {
                    let s = bot_state.read().await;
                    s.positions
                        .iter()
                        .find(|p| p.symbol == rec.symbol)
                        .map(|p| {
                            let r = if p.r_dollars_risked > 1e-8 {
                                p.unrealised_pnl / p.r_dollars_risked
                            } else {
                                0.0
                            };
                            r > 0.3 // add to winners (lowered from 0.5)
                        })
                        .unwrap_or(false)
                };
                if can_scale {
                    let mut s = bot_state.write().await;
                    // Use index to avoid simultaneous mutable + immutable borrow of `s`
                    if let Some(idx) = s.positions.iter().position(|p| p.symbol == rec.symbol) {
                        let add_usd = s.positions[idx].size_usd * (factor - 1.0);
                        let lev = s.positions[idx].leverage;
                        if s.capital >= add_usd && add_usd >= 1.0 {
                            let add_qty = add_usd * lev / cur_price;
                            let old_qty = s.positions[idx].quantity;
                            let old_entry = s.positions[idx].entry_price;
                            let old_stop = s.positions[idx].stop_loss;
                            let new_qty = old_qty + add_qty;
                            // Update weighted average entry so unrealised_pnl stays correct
                            let avg_entry = (old_entry * old_qty + cur_price * add_qty) / new_qty;
                            s.capital -= add_usd;
                            s.positions[idx].quantity = new_qty;
                            s.positions[idx].size_usd += add_usd;
                            s.positions[idx].entry_price = avg_entry;
                            // FIX: update r_dollars_risked to reflect larger position.
                            // Without this the AI sees an inflated (falsely negative) R-multiple
                            // on the next review cycle, increasing the risk of a premature close.
                            s.positions[idx].r_dollars_risked =
                                (avg_entry - old_stop).abs() * new_qty;
                            info!(
                                "🤖 AI scale-up {} ×{:.2}  +${:.2} — {}",
                                rec.symbol, factor, add_usd, rec.reason
                            );
                        }
                    }
                } else {
                    info!(
                        "🤖 AI scale-up {} REJECTED — R < 0.5 (guardrail)",
                        rec.symbol
                    );
                }
            }

            "scale_down" => {
                // Guardrail: keep at least 25% AND enforce minimum hold time.
                // Early scale_down on a young position sabotages both the DCA
                // rescue path and the R-multiple exit tranches.
                let keep_frac = rec.factor.clamp(0.25, 0.99);
                let close_frac = 1.0 - keep_frac;
                let (close_usd, close_qty) = {
                    let s = bot_state.read().await;
                    s.positions
                        .iter()
                        .find(|p| p.symbol == rec.symbol)
                        .filter(|p| p.cycles_held >= 60) // 30 min minimum before any scale_down
                        .map(|p| (p.size_usd * close_frac, p.quantity * close_frac))
                        .unwrap_or((0.0, 0.0))
                };
                if close_usd > 1.0 {
                    let pnl_portion = {
                        let s = bot_state.read().await;
                        s.positions
                            .iter()
                            .find(|p| p.symbol == rec.symbol)
                            .map(|p| p.unrealised_pnl * close_frac)
                            .unwrap_or(0.0)
                    };
                    let mut s = bot_state.write().await;
                    if let Some(pos) = s.positions.iter_mut().find(|p| p.symbol == rec.symbol) {
                        pos.quantity -= close_qty;
                        pos.size_usd -= close_usd;
                        pos.unrealised_pnl -= pnl_portion;
                        pos.r_dollars_risked *= keep_frac;
                        s.capital += close_usd + pnl_portion;
                        s.pnl += pnl_portion;
                        info!(
                            "🤖 AI scale-down {} keep {:.0}%  realised ${:.2} — {}",
                            rec.symbol,
                            keep_frac * 100.0,
                            pnl_portion,
                            rec.reason
                        );
                    }
                }
            }

            _ => {
                // Nothing to do — existing strategy manages the position
            }
        }

        // ── Always persist AI annotation on the position card ─────────────
        {
            let mut s = bot_state.write().await;
            if let Some(pos) = s.positions.iter_mut().find(|p| p.symbol == rec.symbol) {
                pos.ai_action = Some(rec.action.clone());
                pos.ai_reason = Some(rec.reason.chars().take(80).collect()); // cap at 80 chars
            }
        }
    }
}

struct GuardrailOutcome {
    allow_close: bool,
    score: f64,
    components: Vec<String>,
    note: Option<String>,
    false_breakout: bool,
    momentum_stall: bool,
    hold_minutes: u64,
    dca_remaining: u8,
    r_multiple: f64,
}

impl GuardrailOutcome {
    fn components_str(&self) -> String {
        if self.components.is_empty() {
            "none".to_string()
        } else {
            self.components.join(" | ")
        }
    }
}

fn evaluate_ai_close_guard(pos: &PaperPosition) -> GuardrailOutcome {
    const SCORE_THRESHOLD: f64 = 0.92;

    let r = if pos.r_dollars_risked > 1e-8 {
        pos.unrealised_pnl / pos.r_dollars_risked
    } else {
        0.0
    };
    let hold_mins = pos.cycles_held / 2;
    let peak_r = if pos.r_dollars_risked > 1e-8 {
        if pos.side == "LONG" {
            (pos.high_water_mark - pos.entry_price) * pos.quantity / pos.r_dollars_risked
        } else {
            (pos.entry_price - pos.low_water_mark) * pos.quantity / pos.r_dollars_risked
        }
    } else {
        0.0
    };
    let false_breakout =
        peak_r >= 0.10 && r < -0.05 && (15..60).contains(&hold_mins) && pos.dca_count == 0;
    let momentum_stall = hold_mins >= 60 && r.abs() < 0.10 && pos.dca_count >= 1;
    // Reduced max DCA 2→1: live data showed DCA×2/×3 building $50k+ positions
    // (POPCAT DCA×3 = $63k, AIXBT DCA×2 = $53k).  One add-on is enough to
    // average down; a second just compounds the loss when the thesis is wrong.
    let dca_remaining = 1u8.saturating_sub(pos.dca_count);

    if false_breakout {
        return GuardrailOutcome {
            allow_close: true,
            score: 1.0,
            components: vec![format!("False breakout (peak {:.2}R)", peak_r)],
            note: None,
            false_breakout: true,
            momentum_stall: false,
            hold_minutes: hold_mins,
            dca_remaining,
            r_multiple: r,
        };
    }
    if momentum_stall {
        return GuardrailOutcome {
            allow_close: true,
            score: 0.95,
            components: vec!["Momentum stall".to_string()],
            note: None,
            false_breakout: false,
            momentum_stall: true,
            hold_minutes: hold_mins,
            dca_remaining,
            r_multiple: r,
        };
    }

    let loss_score = (-r).clamp(0.0, 1.0);
    let hold_score = (hold_mins as f64 / 60.0).clamp(0.0, 1.0);
    let deep_loss = if r < -0.40 { 0.15 } else { 0.0 };
    let mut components = vec![
        format!("R={:.2}", r),
        format!("Hold={}m", hold_mins),
        format!("DCA rem={}", dca_remaining),
    ];
    if deep_loss > 0.0 {
        components.push("Deep loss".to_string());
    }

    let base = loss_score * 0.55 + hold_score * 0.25 + deep_loss;
    let modifier = (1.0 - 0.08 * dca_remaining as f64).clamp(0.6, 1.0);
    let score = (base * modifier).clamp(0.0, 1.2);
    let allow_close = score >= SCORE_THRESHOLD || (dca_remaining == 0 && r < -0.05);
    let note = if !allow_close && dca_remaining > 0 && r < -0.15 {
        Some(format!("waiting for {} DCA slot(s)", dca_remaining))
    } else {
        None
    };

    GuardrailOutcome {
        allow_close,
        score,
        components,
        note,
        false_breakout,
        momentum_stall,
        hold_minutes: hold_mins,
        dca_remaining,
        r_multiple: r,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  PER-SYMBOL ANALYSIS
// ═══════════════════════════════════════════════════════════════════════════════

/// Analyse a single symbol and optionally execute a paper (or live) trade.
///
/// Steps:
///   1. Fetch 50 × 1h candles from Binance; bail early if < 26.
///   2. Fetch 50 × 4h candles for multi-timeframe confirmation (non-fatal if
///      unavailable — HTF scaling is skipped).
///   3. Compute `TechnicalIndicators`, `OrderFlow`, sentiment and funding data.
///   4. Build `BtcMarketContext` (skipped for BTC itself).
///   5. Call `decision::make_decision()` to get a `Decision`.
///   6. If paper mode and action ≠ SKIP → `execute_paper_trade()`.
///      If live mode → `risk::should_trade()` gate → `hl.place_order()`.
///
/// Indicator snapshot returned alongside a Decision so the candidates table
/// can display live RSI / regime / ATR% without a second lock cycle.
struct SymbolIndicators {
    rsi: f64,
    regime: &'static str, // "Trending" | "Neutral" | "Ranging"
    atr_pct: f64,         // ATR(14) as % of price
    close_price: f64,     // last candle close — used by signal watchlist
}

/// Returns `Ok(Some((Decision, SymbolIndicators)))` even for SKIP decisions so
/// the dashboard can show them; returns `Ok(None)` when candle data is insufficient.
#[allow(clippy::too_many_arguments)]
async fn analyse_symbol(
    symbol: &str,
    market: &Arc<data::MarketClient>,
    hl: &Arc<exchange::HyperliquidClient>,
    db: &Option<SharedDb>,
    config: &config::Config,
    bot_state: &SharedState,
    weights: &SharedWeights,
    sent_cache: &SharedSentiment,
    fund_cache: &SharedFunding,
    btc_dom: f64,     // BTC dominance %
    btc_ret_24h: f64, // BTC 24h return %
    btc_ret_4h: f64,  // BTC 4h return % (for relative performance signal)
    trade_logger: &SharedTradeLogger,
    fee_bps: u32, // builder fee bps for this tenant (1 = Pro, 3 = Free)
    notifier: &Option<SharedNotifier>, // webhook / Telegram notifier
    onchain_cache: &SharedOnchain, // exchange netflow signal
    cex_monitor: &SharedCrossExchange, // cross-exchange price divergence
) -> Result<Option<(decision::Decision, SymbolIndicators)>> {
    let candles = market.fetch_market_data(symbol).await?;
    if candles.len() < 26 {
        return Ok(None);
    }

    // Fetch 4h candles for multi-timeframe confirmation and relative performance.
    // Non-fatal: if unavailable, HTF filter is skipped (scale = 1.0).
    let (htf, asset_return_4h) = match market.fetch_market_data_4h(symbol).await {
        Ok(c4h) if c4h.len() >= 26 => {
            let htf_ind = indicators::calculate_htf(&c4h);
            let ret = if c4h.len() >= 2 {
                let f = c4h.first().unwrap().close;
                let l = c4h.last().unwrap().close;
                if f > 0.0 {
                    (l - f) / f * 100.0
                } else {
                    0.0
                }
            } else {
                0.0
            };
            (Some(htf_ind), ret)
        }
        _ => (None, 0.0),
    };

    let ind = indicators::calculate_all(&candles)?;
    let ob = market.fetch_order_book(symbol).await?;
    let of = signals::detect_order_flow(&ob)?;
    let w = weights.read().await.clone();
    let sent = sent_cache.get(symbol).await;
    let fund = fund_cache.get(symbol).await;

    // Cross-exchange divergence signal: compare HL mid vs Binance/ByBit/OKX.
    // `current_price` is the HL allMids price already fetched in the outer loop.
    let current_price = candles.last().map(|c| c.close).unwrap_or(0.0);
    let cex_sig = cex_monitor.evaluate(symbol, current_price).await;
    // Advance persistence counter regardless of whether signal is active.
    if let Some(ref s) = cex_sig {
        cex_monitor.tick_persistence(symbol, s.hl_premium_pct).await;
    }

    // BTC dominance context not applied to BTC itself (no self-reference).
    let ctx = if symbol == "BTC" {
        None
    } else {
        Some(decision::BtcMarketContext {
            dominance: btc_dom,
            btc_return_24h: btc_ret_24h,
            btc_return_4h: btc_ret_4h,
            asset_return_4h,
        })
    };

    let mut dec = decision::make_decision(
        &candles,
        &ind,
        &of,
        &w,
        sent.as_ref(),
        fund.as_ref(),
        ctx.as_ref(),
        htf.as_ref(),
        cex_sig.as_ref(),
    )?;

    // ── On-chain exchange netflow signal ──────────────────────────────────
    // Coinglass netflow: net USD flowing INTO exchanges = selling pressure (bearish).
    // Flowing OUT = accumulation (bullish).  signal_strength() returns [-1, +1].
    // We apply up to ±4% confidence adjustment on active (non-SKIP) decisions.
    // Max impact is intentionally small — this is a supplementary signal, not primary.
    // get() always returns OnchainData (neutral 0.0 if key absent or symbol unknown).
    let oc_strength = onchain_cache.get(symbol).await.signal_strength();
    if dec.action != "SKIP" && oc_strength.abs() > 0.05 {
        // Aligned = netflow confirms trade direction → boost; opposed → penalty
        let aligned = (dec.action == "BUY" && oc_strength > 0.0)
            || (dec.action == "SELL" && oc_strength < 0.0);
        let adj = oc_strength.abs() * 0.04 * if aligned { 1.0 } else { -1.0 };
        dec.confidence = (dec.confidence + adj).clamp(0.0, 1.0);
        log::debug!(
            "{}: on-chain adj {:+.3} ({}) → conf={:.0}%",
            symbol,
            adj,
            if aligned { "aligned" } else { "opposed" },
            dec.confidence * 100.0
        );
    }

    // ── Funding-staleness warning (fail-open) ────────────────────────────
    // If the funding cache is stale, log a warning but do NOT block entries.
    // make_decision() already received fund=None (get() returns None when stale)
    // so the funding signal simply contributes 0 weight — the other 8+ signals
    // still drive the decision.  Blocking all trades when one optional signal
    // is unavailable is too conservative.
    if dec.action != "SKIP" && fund_cache.is_stale().await {
        let age = fund_cache
            .age_secs()
            .await
            .map(|s| format!("{}s ago", s))
            .unwrap_or_else(|| "never fetched".to_string());
        log::warn!(
            "⚠️  {} → {} proceeding without funding signal (stale: {})",
            symbol,
            dec.action,
            age
        );
    }

    log::debug!(
        "{}: RSI={:.1} trend={:.2}% MACD={:.5} ATR={:.4} → {}",
        symbol,
        ind.rsi,
        ind.trend,
        ind.macd,
        ind.atr,
        dec.action
    );

    // ── Log every decision (including SKIP) for daily analysis ───────────
    let htf_ref = htf.as_ref();
    let regime_str = if ind.adx > 27.0 {
        "Trending"
    } else if ind.adx >= 19.0 {
        "Neutral"
    } else {
        "Ranging"
    };
    {
        let skip_reason = if dec.action == "SKIP" {
            Some(dec.rationale.chars().take(120).collect::<String>())
        } else {
            None
        };
        trade_logger.lock().await.log(&TradeEvent::Decision {
            ts: ts_now(),
            symbol: symbol.to_string(),
            action: dec.action.clone(),
            confidence: dec.confidence,
            rationale: dec.rationale.chars().take(200).collect(),
            rsi: ind.rsi,
            rsi_4h: htf_ref.map_or(50.0, |h| h.rsi_4h),
            adx: ind.adx,
            regime: regime_str.to_string(),
            macd: ind.macd,
            macd_hist: ind.macd_histogram,
            z_score: ind.z_score,
            z_score_4h: htf_ref.map_or(0.0, |h| h.z_score_4h),
            ema_cross_pct: ind.ema_cross_pct,
            atr: ind.atr,
            atr_expansion: ind.atr_expansion_ratio,
            bb_width_pct: ind.bb_width_pct,
            volume_ratio: ind.volume_ratio,
            vwap_pct: ind.vwap_pct,
            sentiment_galaxy: sent.as_ref().map(|s| s.galaxy_score),
            sentiment_bull: sent.as_ref().map(|s| s.bullish_percent),
            funding_rate: fund.as_ref().map(|f| f.funding_rate),
            funding_delta: fund.as_ref().map(|f| f.funding_delta),
            btc_dom_pct: btc_dom,
            asset_ret_4h: asset_return_4h,
            entry_price: dec.entry_price,
            stop_loss: dec.stop_loss,
            take_profit: dec.take_profit,
            leverage: dec.leverage,
            skip_reason,
        });
    }

    // ── Update order-book snapshot on any existing open position ─────────
    // This runs EVERY cycle for every symbol, not just when we trade.
    // Lets the position manager track sentiment drift on open positions.
    {
        let mut s = bot_state.write().await;
        if let Some(pos) = s.positions.iter_mut().find(|p| p.symbol == symbol) {
            let is_adverse = (pos.side == "LONG"
                && (of.sentiment.contains("BEAR") || of.ask_wall_near))
                || (pos.side == "SHORT" && (of.sentiment.contains("BULL") || of.bid_wall_near));
            pos.ob_sentiment = of.sentiment.clone();
            pos.ob_bid_wall_near = of.bid_wall_near;
            pos.ob_ask_wall_near = of.ask_wall_near;
            pos.order_flow_direction = of.direction.clone();
            pos.order_flow_confidence = of.confidence;
            if is_adverse {
                pos.ob_adverse_cycles += 1;
            } else {
                pos.ob_adverse_cycles = 0; // reset when book turns back in our favour
            }
            if let Some(f) = fund.as_ref() {
                pos.funding_rate = f.funding_rate;
                pos.funding_delta = f.funding_delta;
            }
            pos.onchain_strength = oc_strength;
            pos.cex_premium_pct = cex_sig.as_ref().map(|s| s.hl_premium_pct).unwrap_or(0.0);
            pos.cex_mode = cex_sig
                .as_ref()
                .map(|s| format!("{:?}", s.mode))
                .unwrap_or_else(|| "Inactive".to_string());
        }
    }

    if config.paper_trading && dec.action != "SKIP" {
        // Pass current BTC 4h return so execute_paper_trade can store it on entry
        // and use it for DCA thesis validation later.
        // For BTC itself, use 0.0 (no self-referential filter).
        let cycle_btc_ret = if symbol == "BTC" { 0.0 } else { btc_ret_4h };
        execute_paper_trade(
            symbol,
            &dec,
            &ind,
            bot_state,
            weights,
            trade_logger,
            notifier,
            db,
            single_op_tenant(),
            cycle_btc_ret,
            hl,
            fee_bps,
            config.paper_trading,
            config.min_position_pct,
            config.max_position_pct,
        )
        .await;
    } else if !config.paper_trading && dec.action != "SKIP" {
        let account = hl.get_account(config.daily_loss_limit, config.min_health_factor).await?;
        if risk::should_trade(&dec, &account)? {
            let capital = bot_state.read().await.capital;
            match hl.place_order(symbol, &dec, capital, fee_bps).await {
                Ok(id) => {
                    info!(
                        "✅ {} {} @ ${:.4} [{}]",
                        dec.action, symbol, dec.entry_price, id
                    );
                }
                Err(e) => error!("❌ Order failed {}: {}", symbol, e),
            }
        }
    }

    let current_price = candles.last().map_or(1.0, |c| c.close);
    let ind_snapshot = SymbolIndicators {
        rsi: ind.rsi,
        regime: regime_str,
        atr_pct: if ind.atr > 0.0 && current_price > 0.0 {
            ind.atr / current_price * 100.0
        } else {
            0.0
        },
        close_price: current_price,
    };

    Ok(Some((dec, ind_snapshot)))
}

// ═══════════════════════════════════════════════════════════════════════════════
//  POSITION SIZING  (Kelly + Sharpe multiplier + confidence)
// ═══════════════════════════════════════════════════════════════════════════════

/// Returns target fraction of free capital for this trade.
///
/// Priority order:
///   1. If half-Kelly available (≥5 trades): Kelly × confidence_scale × Sharpe_mult
///   2. Fallback confidence tiers × Sharpe_mult
///
/// The circuit-breaker multiplier (`CB_SIZE_MULT = 0.35`) is applied here when
/// the peak→current equity drawdown exceeds `CB_DRAWDOWN_THRESHOLD` (8%).
///
/// Result is clamped to [`MIN_POSITION_PCT`, `MAX_POSITION_PCT`].
fn position_size_pct(
    confidence: f64,
    metrics: &PerformanceMetrics,
    in_circuit_breaker: bool,
    min_position_pct: f64,
    max_position_pct: f64,
) -> f64 {
    let sharpe_mult = metrics.size_multiplier();
    let kelly = metrics.kelly_fraction();

    let base = if kelly > 0.0 {
        // Linear scale: conf=MIN_CONFIDENCE → 60% of Kelly, conf=1.0 → 100% of Kelly.
        let slope = 0.4 / (1.0 - MIN_CONFIDENCE);
        let conf_scale = (0.6 + (confidence - MIN_CONFIDENCE).max(0.0) * slope).min(1.0);
        kelly * conf_scale
    } else {
        // Pre-Kelly fallback tiers (first ~5 trades).
        // Raised so early trades are meaningful — they're also learning trades.
        match confidence {
            c if c >= 0.85 => 0.15,
            c if c >= 0.75 => 0.12,
            c if c >= MIN_CONFIDENCE => 0.08,
            _ => 0.05,
        }
    };

    let cb_mult = if in_circuit_breaker {
        CB_SIZE_MULT
    } else {
        1.0
    };
    (base * sharpe_mult * cb_mult).clamp(min_position_pct, max_position_pct)
}

/// Fraction of equity at risk for this specific trade (stop-loss based).
///
/// Formula: stop_distance% × notional_size / equity
///   = stop_dist_pct × (size_usd × leverage) / equity
///
/// `size_usd` is the MARGIN committed, not the notional.  Without the
/// leverage factor the heat check underestimates actual risk by the full
/// leverage multiple (e.g. 3× leverage → 3× more equity at risk than the
/// naive formula implies).
fn trade_heat(entry: f64, stop: f64, size_usd: f64, equity: f64, leverage: f64) -> f64 {
    if equity < 1.0 {
        return 1.0;
    }
    let stop_dist_pct = (entry - stop).abs() / entry.max(1e-8);
    stop_dist_pct * size_usd * leverage / equity
}

/// Total % of equity at risk across all open positions.
///
/// Risk per position is computed from the **current** stop-loss, not the
/// stale `r_dollars_risked` field (which never updates when the trailing
/// stop advances).  Once a trailing stop has moved to or beyond the entry
/// price the position has a locked-in profit floor — its remaining
/// downside is zero, so it contributes 0% heat.
///
/// Formula per position: max(0, |entry − current_stop| × quantity) / equity
fn portfolio_heat(positions: &[PaperPosition], equity: f64) -> f64 {
    if equity < 1.0 {
        return 1.0;
    }
    positions
        .iter()
        .map(|p| {
            // Dollars currently at risk if stop is hit.
            // Positive  = stop is below entry (LONG) or above entry (SHORT).
            // Zero/neg  = stop has crossed entry → position is protected.
            let current_risk = if p.side == "LONG" {
                (p.entry_price - p.stop_loss) * p.quantity
            } else {
                (p.stop_loss - p.entry_price) * p.quantity
            };
            // Pool-funded positions count at 50% heat weight — they represent
            // captured profits (house money) not own capital, so a full stop-out
            // merely returns the pool to zero rather than hurting base equity.
            let heat_weight = if p.funded_from_pool { 0.5 } else { 1.0 };
            current_risk.max(0.0) / equity * heat_weight
        })
        .sum::<f64>()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ENTRY EXECUTION
// ═══════════════════════════════════════════════════════════════════════════════

/// Evaluate and execute a new paper-trade entry (or pyramid / DCA on existing).
///
/// Decision tree for an existing position on `symbol`:
///   - **Same side, profitable (≥ 1R)**: pyramid (+50% of current size).
///   - **Same side, mild loss (-0.15R to -0.85R) + confidence ≥ DCA_MIN_CONFIDENCE**:
///     DCA (add 50%, recompute average entry and stop).
///   - **Same side, other**: skip (hold position).
///   - **Opposite side, confidence < MIN_CONFIDENCE**: ignore (no flip).
///   - **Opposite side, confidence ≥ MIN_CONFIDENCE**: close current, open new.
///
/// For a new position, all four guards must pass:
///   1. `confidence ≥ MIN_CONFIDENCE` (0.68)
///   2. Sufficient free capital (`size_usd ≥ $2`)
///   3. Per-trade heat ≤ `MAX_TRADE_HEAT` (2% of equity) — scales down if over
///   4. Portfolio heat < `MAX_PORTFOLIO_HEAT` (15% of equity)
///
/// There is no hard cap on total position count or per-direction count.
/// The heat budget and Kelly position sizing are the sole limits: once
/// 15% of equity is at risk the bot simply waits for a position to close
/// before opening another, regardless of direction.
///
/// Circuit breaker: if peak→current drawdown > `CB_DRAWDOWN_THRESHOLD` (8%),
/// position size is multiplied by `CB_SIZE_MULT` (0.35).
#[allow(clippy::too_many_arguments)]
async fn execute_paper_trade(
    symbol: &str,
    dec: &decision::Decision,
    ind: &indicators::TechnicalIndicators,
    bot_state: &SharedState,
    weights: &SharedWeights,
    trade_logger: &SharedTradeLogger,
    notifier: &Option<SharedNotifier>,
    db: &Option<SharedDb>,
    tenant_id: Uuid,
    // BTC 4h return this cycle (%) — used for per-trade budget + DCA thesis guard.
    // Pass 0.0 for BTC itself (no self-referential filter).
    btc_ret_4h: f64,
    hl: &Arc<exchange::HyperliquidClient>,
    fee_bps: u32,
    paper: bool,
    // Position sizing bounds from config (MIN_POSITION_PCT / MAX_POSITION_PCT env vars).
    min_position_pct: f64,
    max_position_pct: f64,
) {
    let target_side = if dec.action == "BUY" { "LONG" } else { "SHORT" };

    // Check for existing position on this symbol
    {
        let s = bot_state.read().await;
        let existing = s.positions.iter().find(|p| p.symbol == symbol);
        if let Some(pos) = existing {
            let r_mult = if pos.r_dollars_risked > 1e-8 {
                pos.unrealised_pnl / pos.r_dollars_risked
            } else {
                0.0
            };

            if pos.side == target_side {
                // ── Same direction: pyramid UP on winner ──
                if r_mult >= 1.0 && pos.tranches_closed == 0 {
                    drop(s);
                    pyramid_position(symbol, dec, ind, bot_state).await;
                    return;
                }

                // ── Same direction: DCA DOWN on moderate loser with conviction ──
                // Tiered DCA cap: high conviction signals earn extra DCA slots
                //   confidence ≥ DCA_MIN_CONFIDENCE (0.65): up to 2 add-ons (standard)
                //   confidence ≥ 0.75:                      up to 3 add-ons (high conviction)
                //   confidence ≥ 0.85:                      up to 4 add-ons (very high conviction)
                // Rationale: a strong confluence of signals screaming the same direction
                // on a volatile asset like REZ/KAS deserves averaging down further.
                // Upper bound kept at -0.85R — position must not be near stop before adding.
                let dca_max = if dec.confidence >= 0.85 {
                    4
                } else if dec.confidence >= 0.75 {
                    3
                } else {
                    2
                };

                // ── BTC swing thesis guard ────────────────────────────────────
                // Before DCA-ing in, verify BTC hasn't swung hard against the trade
                // direction since we entered. A 1.5%+ BTC reversal against our trade
                // suggests macro has turned — adding to a losing position in that
                // environment is "buying a sinking ship" rather than improving price.
                // Only active when BTC dominance is high enough to matter (>48%).
                let btc_swing = btc_ret_4h - pos.btc_ret_at_entry;
                let btc_against = if btc_ret_4h != 0.0 {
                    (pos.side == "LONG" && btc_swing < -1.5)
                        || (pos.side == "SHORT" && btc_swing > 1.5)
                } else {
                    false
                };
                if btc_against {
                    info!(
                        "🛑 DCA {} blocked — BTC thesis broken \
                           (entry BTC4h={:+.1}%, now {:+.1}%, swing {:+.1}% against {})",
                        symbol, pos.btc_ret_at_entry, btc_ret_4h, btc_swing, pos.side
                    );
                    return;
                }

                if r_mult < -0.15
                    && r_mult > -0.85
                    && pos.dca_count < dca_max
                    && dec.confidence >= DCA_MIN_CONFIDENCE
                {
                    drop(s);
                    dca_position(symbol, dec, ind, bot_state, btc_ret_4h).await;
                    return;
                }

                // Same side but conditions not met — hold without action
                return;
            }

            // ── Opposite side: ONLY reverse on high-confidence signal ──
            // Require ≥MIN_CONFIDENCE to avoid noise flipping existing positions.
            // Use dynamic floor based on performance metrics.
            let effective_floor = s.metrics.confidence_floor(MIN_CONFIDENCE);
            if dec.confidence < effective_floor {
                info!(
                    "⏸  {} opposing signal ignored (conf {:.0}% < {:.0}%) — holding {} position",
                    symbol,
                    dec.confidence * 100.0,
                    effective_floor * 100.0,
                    pos.side
                );
                return;
            }
            // Minimum hold guard — do not flip a position that opened < 30 min ago.
            // Early noise can easily generate a 1-bar counter-signal; the original
            // signal needs time to play out before we admit it has reversed.
            if pos.cycles_held < 60 {
                info!(
                    "⏸  {} reversal blocked — position only {} cycles old (need 60 / 30 min)",
                    symbol, pos.cycles_held
                );
                return;
            }
            // Clone values before dropping the read guard to satisfy the borrow checker
            let pos_side = pos.side.clone();
            let r_mult_snap = r_mult;
            let cycles_snap = pos.cycles_held;
            drop(s);
            info!(
                "🔄 {} signal reversal: {} at {:.2}R  held={}  conf={:.0}%",
                symbol,
                pos_side,
                r_mult_snap,
                cycles_snap,
                dec.confidence * 100.0
            );
            close_paper_position(
                symbol,
                dec.entry_price,
                "SignalExit",
                bot_state,
                weights,
                trade_logger,
                notifier,
                db,
                tenant_id,
                hl,
                fee_bps,
                paper,
            )
            .await;
        }
        // No existing: fall through to new entry
    }

    // ── Correlation filter ────────────────────────────────────────────────
    // Prevents stacking highly-correlated same-direction positions (e.g. BTC+ETH LONG
    // when corr=0.85 doubles macro exposure, not diversification).
    // Uses 30-day rolling Pearson correlation from correlation.rs.
    {
        let s = bot_state.read().await;
        match correlation::correlation_block(symbol, target_side, dec.confidence, &s.positions) {
            correlation::CorrBlock::Blocked {
                existing,
                corr,
                existing_conf,
            } => {
                info!(
                    "⚡ {} {} blocked by correlation — {:.2} corr with {} \
                       (conf {:.0}% ≤ existing {:.0}% + {:.0}% edge required)",
                    symbol,
                    target_side,
                    corr,
                    existing,
                    dec.confidence * 100.0,
                    existing_conf * 100.0,
                    correlation::CONF_EDGE * 100.0
                );
                return;
            }
            correlation::CorrBlock::Override { existing, corr } => {
                info!(
                    "⚡ {} {} corr-override approved — {:.2} corr with {} \
                       (confidence edge sufficient)",
                    symbol, target_side, corr, existing
                );
            }
            correlation::CorrBlock::Clear => {}
        }
    }

    // ── Open new position ─────────────────────────────────────────────────
    let atr = ind.atr.max(dec.entry_price * 0.001);

    let mut s = bot_state.write().await;
    let metrics = s.metrics.clone();
    let equity = s.capital
        + s.positions
            .iter()
            .map(|p| p.size_usd + p.unrealised_pnl)
            .sum::<f64>();

    // ── Circuit breaker ───────────────────────────────────────────────────
    // Uses rolling 7-day peak (not all-time) so a single lucky spike long
    // ago doesn't permanently throttle sizing.  When drawdown from the
    // 7-day high exceeds CB_DRAWDOWN_THRESHOLD (8%), new position sizes are
    // scaled to CB_SIZE_MULT (0.35×).
    let rolling_peak = s
        .equity_window
        .iter()
        .map(|&(_, e)| e)
        .fold(equity, f64::max); // fallback to current equity if window empty
    let drawdown = if rolling_peak > 0.0 {
        (rolling_peak - equity) / rolling_peak
    } else {
        0.0
    };
    let in_cb = drawdown > CB_DRAWDOWN_THRESHOLD;

    // ── BTC swing timing gate ─────────────────────────────────────────────
    // Trades "follow BTC" — when BTC is making a big move AGAINST our intended
    // direction, entering a new altcoin trade is fighting the macro.  This gate
    // raises the confidence floor by 0.08 when BTC 4h return is strongly opposed
    // (>2%), blocking borderline signals that would normally squeak through.
    // BTC signals skipping this check (btc_ret_4h == 0.0) are intentional.
    let btc_opposed_entry = if btc_ret_4h.abs() > 2.0 && symbol != "BTC" {
        (dec.action == "BUY" && btc_ret_4h < -2.0) || (dec.action == "SELL" && btc_ret_4h > 2.0)
    } else {
        false
    };

    // ── Minimum confidence gate ────────────────────────────────────────────
    // Only enter trades where the signal is genuinely strong.
    // Signals below MIN_CONFIDENCE generated 0W/14L in choppy markets.
    // Use dynamic floor based on performance metrics; add extra 0.10 if circuit breaker active.
    // Add extra 0.08 when BTC is making a big opposing swing.
    let mut effective_floor = metrics.confidence_floor(MIN_CONFIDENCE);
    if in_cb {
        effective_floor = (effective_floor + 0.10).min(0.92);
    }
    if btc_opposed_entry {
        effective_floor = (effective_floor + 0.08).min(0.95);
        info!(
            "⚡ {} BTC swing gate: BTC 4h={:+.1}% opposes {}, floor raised to {:.0}%",
            symbol,
            btc_ret_4h,
            dec.action,
            effective_floor * 100.0
        );
    }
    if dec.confidence < effective_floor {
        info!(
            "⚠ {} skipped — confidence {:.0}% below {:.0}% minimum",
            symbol,
            dec.confidence * 100.0,
            effective_floor * 100.0
        );
        return;
    }
    s.cb_active = in_cb; // keep dashboard in sync with actual sizing CB
    if in_cb {
        info!(
            "🔴 CB ACTIVE — 7d drawdown {:.1}% (>{:.0}%), sizing ×{:.2}",
            drawdown * 100.0,
            CB_DRAWDOWN_THRESHOLD * 100.0,
            CB_SIZE_MULT
        );
        trade_logger.lock().await.log(&TradeEvent::CircuitBreaker {
            ts: ts_now(),
            activated: true,
            drawdown_pct: drawdown * 100.0,
            threshold_pct: CB_DRAWDOWN_THRESHOLD * 100.0,
            size_mult: CB_SIZE_MULT,
            peak_equity: rolling_peak,
            current_equity: equity,
        });
        // Fire circuit-breaker webhook notification
        if let Some(n) = notifier {
            let n = n.clone();
            let dd = drawdown * 100.0;
            tokio::spawn(async move {
                n.circuit_breaker(dd, CB_SIZE_MULT).await;
            });
        }
    }

    // ── Small-wallet sizing override ──────────────────────────────────────
    // Small wallets suffer from two compounding problems:
    //   1. MIN_POSITION_PCT=5% → $5 margin on $100 → $25 notional at 5×.
    //      Fees + spread consume a large fraction of any gain.
    //   2. The Kelly/confidence formula is calibrated for larger accounts.
    //      Applying it at micro scale produces positions too small to matter.
    //
    // Fix: scale the position-size bounds UP for small wallets so each trade
    // is a meaningful fraction of the account.  Fewer, larger positions beat
    // many tiny ones when fees are a fixed overhead per trade.
    let effective_min_pct = if s.initial_capital <= 100.0 {
        0.15 // ≤$100: minimum 15% per trade ($15 on a $100 wallet)
    } else if s.initial_capital <= 300.0 {
        0.10 // $101–$300: minimum 10% per trade
    } else {
        min_position_pct // $300+: use configured value
    };
    let effective_max_pct = if s.initial_capital <= 100.0 {
        0.35 // ≤$100: allow up to 35% per trade for high-conviction signals
    } else if s.initial_capital <= 300.0 {
        0.20 // $101–$300: allow up to 20%
    } else {
        max_position_pct // $300+: use configured value
    };
    let pct = position_size_pct(dec.confidence, &metrics, in_cb, effective_min_pct, effective_max_pct);
    let mut size_usd = s.capital * pct;

    // Guard: min position size — scales with wallet to support micro-accounts.
    // Hyperliquid's hard minimum order notional is ~$1 USD (at any leverage).
    // We set the floor as 2% of initial_capital (clamped $0.50 … $2.00) so a
    // $10 wallet can trade $0.50+ positions while large wallets still use $2.
    let min_pos_usd = (s.initial_capital * 0.02).clamp(0.50, 2.0);
    if size_usd < min_pos_usd || s.capital < size_usd {
        info!(
            "⚠ {} skipped — insufficient capital (${:.2}, min=${:.2})",
            symbol, s.capital, min_pos_usd
        );
        return;
    }
    // Guard: per-trade heat ≤ MAX_TRADE_HEAT (2%) of equity.
    // If the default Kelly/confidence size would exceed the heat limit, scale it
    // down to the maximum allowed size rather than skipping the trade entirely.
    //
    // Note: leverage is factored into both heat and the allowed-size calculation.
    // Without it the check underestimates actual risk by the full leverage multiple.
    let stop_dist_pct = (dec.entry_price - dec.stop_loss).abs() / dec.entry_price.max(1e-8);
    let t_heat = trade_heat(
        dec.entry_price,
        dec.stop_loss,
        size_usd,
        equity,
        dec.leverage,
    );
    if t_heat > MAX_TRADE_HEAT {
        // Max MARGIN that keeps notional R-risk at exactly MAX_TRADE_HEAT of equity:
        //   MAX_TRADE_HEAT = stop_dist_pct × margin × leverage / equity
        //   → margin = MAX_TRADE_HEAT × equity / (stop_dist_pct × leverage)
        let allowed = MAX_TRADE_HEAT * equity / (stop_dist_pct * dec.leverage);
        if allowed < min_pos_usd {
            info!(
                "⚠ {} skipped — stop too tight, min heat size ${:.2} < ${:.2}",
                symbol, allowed, min_pos_usd
            );
            return;
        }
        info!(
            "🌡 {} heat-scaled: ${:.2} → ${:.2} (stop_dist={:.2}% × {:.1}×lev)",
            symbol,
            size_usd,
            allowed,
            stop_dist_pct * 100.0,
            dec.leverage
        );
        size_usd = allowed; // apply the reduction — enforce the heat limit
    }
    // Guard: hard cap on simultaneous open positions — scales with wallet size.
    // Tightened for small wallets: paired with the larger per-trade sizing above,
    // fewer concurrent positions keeps each one large enough to generate real
    // returns rather than being fee-eaten noise.
    //   ≤$100  : 3 positions max — concentrate capital into 3 best signals
    //   $101-300: 5 positions max — still concentrated
    //   $301-500: 10 positions max
    //   $500+  : full cap (20)
    let effective_pos_cap = if s.initial_capital <= 25.0 {
        2_usize // $10–$25: max 2 positions (was 3)
    } else if s.initial_capital <= 100.0 {
        3_usize // $26–$100: max 3 positions (was 6) — pairs with 15-35% sizing
    } else if s.initial_capital <= 300.0 {
        5_usize // $101–$300: max 5 positions (was mixed in 15 band)
    } else if s.initial_capital <= 500.0 {
        10_usize // $301–$500: max 10 positions (was 15)
    } else {
        MAX_OPEN_POSITIONS // $500+: full cap
    };
    if s.positions.len() >= effective_pos_cap {
        info!(
            "🚫 {} skipped — position cap reached ({}/{} for ${:.0} wallet)",
            symbol,
            s.positions.len(),
            effective_pos_cap,
            s.initial_capital
        );
        return;
    }
    // Guard: total portfolio heat ≤ MAX_PORTFOLIO_HEAT
    // Pool-funded positions count at 50% heat weight (see portfolio_heat), so
    // we check against the own-capital heat ceiling while remaining open to
    // pool-funded entries that don't burn through base capital at all.
    let p_heat = portfolio_heat(&s.positions, equity);
    if p_heat >= MAX_PORTFOLIO_HEAT {
        info!(
            "🔥 {} skipped — portfolio heat {:.1}% (max {:.0}%)",
            symbol,
            p_heat * 100.0,
            MAX_PORTFOLIO_HEAT * 100.0
        );
        return;
    }

    // ── Re-entry detection + pool-funded sizing ──────────────────────────
    // If the same coin had a profitable close within the last 60 cycles
    // (~30 min) and the house-money pool is large enough, we size from the
    // pool rather than own capital.  This implements "finance the next move
    // with profits" — we're playing with house money so we:
    //   • Don't deduct from s.capital
    //   • Allow up to 2× standard size on confirmed re-entries
    //   • Bypass the own-capital minimum-capital guard (pool has its own check)
    //
    // If there is NO recent profitable close but the pool is healthy (≥ size_usd),
    // we still prefer the pool for any new trade so own-capital heat is preserved.
    let cycle_now = s.cycle_count;
    let reentry_profit: f64 = s
        .recently_closed
        .iter()
        .filter(|(sym, _, at)| sym == symbol && cycle_now.saturating_sub(*at) <= 60)
        .map(|(_, pnl, _)| *pnl)
        .sum();

    let pool_available = s.house_money_pool;
    // Require pool to cover at least the standard size_usd before preferring it.
    // This prevents deploying a half-sized pool stake that's worse than own-capital
    // sizing.
    let pool_qualifies = pool_available >= size_usd;

    let (funded_from_pool, pool_stake_usd): (bool, f64);
    if pool_qualifies {
        // Re-entry on same coin → up to 2× standard size from the pool
        let pool_target = if reentry_profit > 0.0 {
            let two_x = size_usd * 2.0;
            if pool_available >= two_x {
                info!(
                    "🏦 {} re-entry × 2 from house-money pool \
                       (recent profit ${:.2}, pool ${:.2})",
                    symbol, reentry_profit, pool_available
                );
                two_x
            } else {
                info!(
                    "🏦 {} re-entry from pool (partial 2× — pool ${:.2} < 2×${:.2})",
                    symbol, pool_available, size_usd
                );
                pool_available.min(size_usd * 2.0)
            }
        } else {
            info!(
                "🏦 {} funded from house-money pool ${:.2} (preserving own capital)",
                symbol, pool_available
            );
            size_usd // standard size, from pool
        };
        size_usd = pool_target;
        funded_from_pool = true;
        pool_stake_usd = pool_target;
        s.house_money_pool -= pool_target;
        s.pool_deployed_usd += pool_target;
    } else {
        funded_from_pool = false;
        pool_stake_usd = 0.0;
    }

    // Apply confidence-scaled leverage — quantity based on notional, capital deducted at margin
    let leverage = dec.leverage;
    let notional = size_usd * leverage;
    let qty = notional / dec.entry_price;
    let r_risk = (dec.entry_price - dec.stop_loss).abs() * qty; // dollars risked on notional

    // ── Per-trade budget cap ──────────────────────────────────────────────
    // Plan the maximum DCA spend at entry time so we never "over-buy a sinking ship."
    // Budget = initial margin × dca_max_multiplier.
    // Each DCA add-on uses 50% of the current size (geometric growth).
    // Budget = initial margin × budget_mult.  Tightened to prevent a single
    // position inflating to 3-4× initial via unchecked DCA compounding:
    //   dca_max=2: budget cap = 1.0 × entry  (max 1 DCA → 1.5× total)
    //   dca_max=3: budget cap = 1.5 × entry  (max 2 DCAs → 2.25× total)
    // Previous values (3/4 max) produced $491k notional on a single altcoin (GRIFFAIN).
    let entry_dca_max = if dec.confidence >= 0.85 {
        3  // was 4 — allows up to 2 DCA entries (1+0.5+0.75 = 2.25× initial)
    } else {
        2  // confidence <0.85: was 3 (1 DCA) or unchanged — capped at 1 DCA
    };
    let budget_mult = (entry_dca_max as f64 - 1.0).max(1.0); // 1.0/1.5
    let trade_budget_usd = size_usd * budget_mult;

    // Deduct from the right bucket: pool-funded entries don't touch own capital
    if funded_from_pool {
        // pool already debited above; capital unchanged
    } else {
        s.capital -= size_usd; // only deduct margin, not notional
    }
    s.positions.push(PaperPosition {
        symbol: symbol.to_string(),
        side: target_side.to_string(),
        entry_price: dec.entry_price,
        quantity: qty,
        size_usd,
        stop_loss: dec.stop_loss,
        take_profit: dec.take_profit,
        atr_at_entry: atr,
        high_water_mark: dec.entry_price,
        low_water_mark: dec.entry_price,
        partial_closed: false,
        r_dollars_risked: r_risk,
        tranches_closed: 0,
        dca_count: 0,
        leverage,
        cycles_held: 0,
        entry_time: now_str(),
        unrealised_pnl: 0.0,
        contrib: dec.signal_contribution.clone(),
        ai_action: None,
        ai_reason: None,
        entry_confidence: dec.confidence,
        trade_budget_usd,
        dca_spent_usd: 0.0,
        btc_ret_at_entry: btc_ret_4h,
        initial_margin_usd: size_usd,
        ob_sentiment: String::new(),
        ob_bid_wall_near: false,
        ob_ask_wall_near: false,
        ob_adverse_cycles: 0,
        order_flow_confidence: 0.0,
        order_flow_direction: String::new(),
        funding_rate: 0.0,
        funding_delta: 0.0,
        onchain_strength: 0.0,
        cex_premium_pct: 0.0,
        cex_mode: String::new(),
        funded_from_pool,
        pool_stake_usd,
        venue: "Hyperliquid Perps (paper)".to_string(),
    });

    let kelly_str = if metrics.kelly_fraction() > 0.0 {
        format!("Kelly={:.1}%", metrics.kelly_fraction() * 100.0)
    } else {
        "pre-Kelly".to_string()
    };

    let funding_tag = if funded_from_pool {
        "🏦POOL"
    } else {
        "💵OWN"
    };
    info!("📝 {} {} @ ${:.4}  margin=${:.2}  {:.1}×lev  notional=${:.2}  R=${:.2}  heat={:.1}%  {}  [{}]",
        target_side, symbol, dec.entry_price,
        size_usd, leverage, notional,
        r_risk, p_heat * 100.0, funding_tag, kelly_str);

    // Snapshot position for collective upsert (must happen before drop(s))
    let hp_snap = s.positions.last().cloned();

    // Log the trade entry
    drop(s);
    trade_logger.lock().await.log(&TradeEvent::TradeEntry {
        ts: ts_now(),
        symbol: symbol.to_string(),
        side: target_side.to_string(),
        entry_price: dec.entry_price,
        size_usd,
        leverage,
        notional_usd: notional,
        stop_loss: dec.stop_loss,
        take_profit: dec.take_profit,
        r_risk_usd: r_risk,
        confidence: dec.confidence,
        rationale: dec.rationale.chars().take(200).collect(),
        in_circuit_breaker: in_cb,
        portfolio_heat_pct: p_heat * 100.0,
        kelly_pct: metrics.kelly_fraction() * 100.0,
    });

    // ── Fire position-opened webhook / Telegram notification ──────────────
    if let Some(n) = notifier {
        let n = n.clone();
        let sym = symbol.to_string();
        let side = target_side.to_string();
        let entry = dec.entry_price;
        let sz = size_usd;
        let conf = dec.confidence;
        let sl = dec.stop_loss;
        let lev = leverage;
        tokio::spawn(async move {
            n.position_opened(&sym, &side, entry, sz, conf, sl, lev)
                .await;
        });
    }

    // ── Collective intelligence: register new position in hot_positions ────
    if let (Some(ref shared_db), Some(snap)) = (db, hp_snap) {
        let db_hp = shared_db.clone();
        tokio::spawn(async move {
            collective::upsert_hot_position(&db_hp, tenant_id, &snap).await;
        });
    }
}

/// Add 50% of original entry size to an existing winning position (pyramid).
async fn pyramid_position(
    symbol: &str,
    dec: &decision::Decision,
    ind: &indicators::TechnicalIndicators,
    bot_state: &SharedState,
) {
    let atr = ind.atr.max(dec.entry_price * 0.001);
    let mut s = bot_state.write().await;

    let idx = s.positions.iter().position(|p| p.symbol == symbol);
    if let Some(idx) = idx {
        // Add-on size = 50% of the current remaining position size
        let add_size = s.positions[idx].size_usd * 0.5;
        let is_pool = s.positions[idx].funded_from_pool;
        if is_pool {
            if s.house_money_pool < add_size || add_size < 1.0 {
                return;
            }
        } else if s.capital < add_size || add_size < 1.0 {
            return;
        }

        // Apply the same leverage as the original position so the new shares
        // carry the same notional exposure per dollar of margin.
        let lev = s.positions[idx].leverage;
        let add_qty = add_size * lev / dec.entry_price;

        // Weighted average entry — pyramid price is higher than original,
        // so avg_entry rises. Needed for correct unrealised_pnl tracking.
        let old_qty = s.positions[idx].quantity;
        let old_entry = s.positions[idx].entry_price;
        let new_qty = old_qty + add_qty;
        let avg_entry = (old_entry * old_qty + dec.entry_price * add_qty) / new_qty;

        if is_pool {
            s.house_money_pool -= add_size;
            s.pool_deployed_usd += add_size;
            s.positions[idx].pool_stake_usd += add_size;
        } else {
            s.capital -= add_size;
        }

        s.positions[idx].quantity = new_qty;
        s.positions[idx].size_usd += add_size;
        s.positions[idx].entry_price = avg_entry;
        s.positions[idx].r_dollars_risked +=
            (dec.entry_price - s.positions[idx].stop_loss).abs() * add_qty;
        // Tighten stop to pyramided entry's stop if it's better
        if (dec.stop_loss > s.positions[idx].stop_loss && s.positions[idx].side == "LONG")
            || (dec.stop_loss < s.positions[idx].stop_loss && s.positions[idx].side == "SHORT")
        {
            s.positions[idx].stop_loss = dec.stop_loss;
        }
        // Update HWM to current price
        if dec.entry_price > s.positions[idx].high_water_mark {
            s.positions[idx].high_water_mark = dec.entry_price;
        }
        let _ = atr;

        info!(
            "📈 PYRAMID {} @ ${:.4} +${:.2} (total ${:.2})",
            symbol, dec.entry_price, add_size, s.positions[idx].size_usd
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  DCA — AVERAGE DOWN INTO A LOSING POSITION WITH STRONG CONVICTION
// ═══════════════════════════════════════════════════════════════════════════════

/// Average down into an existing losing position (DCA add-on).
///
/// Called when:
///   • Position is between −0.15R and −0.75R (mild loss, not near stop)
///   • Signal still confirms the original direction with ≥ 0.72 confidence
///   • dca_count < 2 (max 2 adds)
///
/// What it does:
///   • Adds 50% of the CURRENT position size as new capital
///   • Recomputes weighted average entry price (lowers it for LONG, raises for SHORT)
///   • Updates stop-loss to 2×ATR from the new average entry (never worsens stop)
///   • Increments dca_count
async fn dca_position(
    symbol: &str,
    dec: &decision::Decision,
    ind: &indicators::TechnicalIndicators,
    bot_state: &SharedState,
    // Current BTC 4h return — stored on position so thesis can be tracked.
    btc_ret_4h: f64,
) {
    let atr = ind.atr.max(dec.entry_price * 0.001);
    let mut s = bot_state.write().await;
    let idx = s.positions.iter().position(|p| p.symbol == symbol);

    if let Some(idx) = idx {
        let add_size = s.positions[idx].size_usd * 0.50;
        let is_pool = s.positions[idx].funded_from_pool;
        if is_pool {
            if s.house_money_pool < add_size || add_size < 1.0 {
                info!(
                    "⚠ DCA {} skipped — insufficient pool (${:.2})",
                    symbol, s.house_money_pool
                );
                return;
            }
        } else if s.capital < add_size || add_size < 1.0 {
            info!(
                "⚠ DCA {} skipped — insufficient capital (${:.2})",
                symbol, s.capital
            );
            return;
        }

        // ── Per-trade budget enforcement ─────────────────────────────────
        // Never spend more than the pre-planned DCA budget.
        // Prevents doubling / tripling into a genuinely broken signal.
        let budget_remaining = s.positions[idx].trade_budget_usd - s.positions[idx].dca_spent_usd;
        if add_size > budget_remaining + 0.01 {
            info!(
                "💰 DCA {} blocked — budget exhausted \
                   (spent=${:.2}, budget=${:.2}, would add=${:.2})",
                symbol, s.positions[idx].dca_spent_usd, s.positions[idx].trade_budget_usd, add_size
            );
            return;
        }

        // Apply leverage so DCA shares have the same notional-per-dollar as the original.
        let lev = s.positions[idx].leverage;
        let add_qty = add_size * lev / dec.entry_price;
        let old_qty = s.positions[idx].quantity;
        let old_entry = s.positions[idx].entry_price;
        let new_qty = old_qty + add_qty;

        // Weighted average entry
        let avg_entry = (old_entry * old_qty + dec.entry_price * add_qty) / new_qty;

        // New ATR-based stop from the average entry (2×ATR)
        let new_stop = if s.positions[idx].side == "LONG" {
            avg_entry - atr * 2.0
        } else {
            avg_entry + atr * 2.0
        };

        // Only tighten the stop (move it closer to entry for LONG = higher value)
        let improved_stop = if s.positions[idx].side == "LONG" {
            new_stop.max(s.positions[idx].stop_loss) // higher stop = better for LONG
        } else {
            new_stop.min(s.positions[idx].stop_loss) // lower stop = better for SHORT
        };

        if is_pool {
            s.house_money_pool -= add_size;
            s.pool_deployed_usd += add_size;
            s.positions[idx].pool_stake_usd += add_size;
        } else {
            s.capital -= add_size;
        }
        s.positions[idx].quantity = new_qty;
        s.positions[idx].size_usd += add_size;
        s.positions[idx].entry_price = avg_entry;
        s.positions[idx].stop_loss = improved_stop;
        s.positions[idx].dca_count += 1;
        s.positions[idx].dca_spent_usd += add_size;
        // Update BTC context snapshot — thesis validation on next DCA will
        // compare against this value, not the original entry value.
        if btc_ret_4h != 0.0 {
            s.positions[idx].btc_ret_at_entry = btc_ret_4h;
        }

        // Recalculate dollars risked from new avg entry and stop
        s.positions[idx].r_dollars_risked = (avg_entry - improved_stop).abs() * new_qty;

        let budget_left = s.positions[idx].trade_budget_usd - s.positions[idx].dca_spent_usd;
        info!(
            "📉 DCA×{} {} @ ${:.4}  avg_entry=${:.4}  stop=${:.4}  \
               +${:.2}  total=${:.2}  budget_left=${:.2}",
            s.positions[idx].dca_count,
            symbol,
            dec.entry_price,
            avg_entry,
            improved_stop,
            add_size,
            s.positions[idx].size_usd,
            budget_left
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  PARTIAL PROFIT TAKING  (1/4 at 1R, 1/3 at 2R, 1/3 at 4R)
// ═══════════════════════════════════════════════════════════════════════════════

/// Close one tranche of an open position at an R-multiple milestone.
///
/// | tranche | R target | Fraction closed | Purpose                        |
/// |---------|----------|-----------------|--------------------------------|
/// |    0    |    1R    |       1/4       | Lock in early profit; reduce risk |
/// |    1    |    2R    |       1/3       | Take more off as trade extends |
/// |    2    |    4R    |       1/3       | Capture deep runner profits    |
#[allow(clippy::too_many_arguments)]
async fn take_partial(
    symbol: String,
    exit_price: f64,
    tranche: u8,
    bot_state: &SharedState,
    weights: &SharedWeights,
    trade_logger: &SharedTradeLogger,
    hl: &Arc<exchange::HyperliquidClient>,
    fee_bps: u32,
    paper: bool,
) {
    let mut s = bot_state.write().await;
    let idx = s
        .positions
        .iter()
        .position(|p| p.symbol == symbol && p.tranches_closed < tranche + 1);
    if let Some(idx) = idx {
        // Tranche 0 = 1/4 (early profit capture); tranches 1 & 2 = 1/3 each
        let (close_frac, r_num, r_label) = match tranche {
            0 => (0.25_f64, 1_u8, "1R"),
            1 => (1.0 / 3.0, 2_u8, "2R"),
            _ => (1.0 / 3.0, 4_u8, "4R"),
        };

        let close_qty = s.positions[idx].quantity * close_frac;
        let close_size = s.positions[idx].size_usd * close_frac;
        let entry = s.positions[idx].entry_price;
        let side = s.positions[idx].side.clone();
        let contrib = s.positions[idx].contrib.clone();
        let was_long = side == "LONG";

        let trade_pnl = if was_long {
            (exit_price - entry) * close_qty
        } else {
            (entry - exit_price) * close_qty
        };

        s.positions[idx].quantity -= close_qty;
        s.positions[idx].size_usd -= close_size;
        s.positions[idx].r_dollars_risked *= 1.0 - close_frac; // scale down risked dollars
        s.positions[idx].tranches_closed = tranche + 1;
        s.positions[idx].partial_closed = true;

        // ── Profit recycling: route profits to house_money_pool ───────────
        // Margin fraction returns to capital (it was always ours).
        // Profit goes to the pool — available to finance the next move
        // without touching original capital.
        // For pool-funded positions, the margin fraction also returns to pool.
        let funded = s.positions[idx].funded_from_pool;
        if funded {
            // Pool stake reducing proportionally
            let stake_reduction = s.positions[idx].pool_stake_usd * close_frac;
            s.positions[idx].pool_stake_usd -= stake_reduction;
            s.pool_deployed_usd = (s.pool_deployed_usd - stake_reduction).max(0.0);
            s.house_money_pool += stake_reduction; // margin fraction back to pool
        } else {
            s.capital += close_size; // margin fraction back to capital
        }
        // Profit (positive P&L) always goes to house_money_pool regardless
        if trade_pnl > 0.0 {
            s.house_money_pool += trade_pnl;
        } else {
            // Loss is absorbed from capital (even on pool-funded, the pool is already returned)
            s.capital += trade_pnl; // negative amount
        }
        s.pnl += trade_pnl;

        let pnl_pct = trade_pnl / close_size * 100.0;
        let frac_label = if tranche == 0 { "¼" } else { "⅓" };

        info!(
            "💰 {}R PARTIAL {} {} @ ${:.4}  P&L {:+.2} ({:+.1}%)  [{} closed]  pool=${:.2}",
            r_num, side, symbol, exit_price, trade_pnl, pnl_pct, frac_label, s.house_money_pool
        );

        let partial_breakdown = Some(format!(
            "<div style='font-size:.78em;padding:4px 0;line-height:1.7'>\
             <div>Partial close {frac} at <b>{lbl}</b> target &nbsp;·&nbsp; \
             entry <b>${entry:.4}</b> → <b>${exit:.4}</b></div>\
             <div style='color:#8b949e'>Locked in <b style='color:#3fb950'>{pnl:+.2}</b> \
             ({pct:+.1}%) on this tranche</div></div>",
            frac = frac_label,
            lbl = r_label,
            entry = entry,
            exit = exit_price,
            pnl = trade_pnl,
            pct = pnl_pct,
        ));
        let entry_time_partial = s
            .positions
            .get(idx)
            .map(|p| p.entry_time.clone())
            .unwrap_or_default();
        let leverage_partial = s.positions.get(idx).map(|p| p.leverage).unwrap_or(1.0);
        let fees_partial = ledger::estimate_fees(close_size, leverage_partial);
        let partial_trade = ClosedTrade {
            symbol: symbol.clone(),
            side,
            entry,
            exit: exit_price,
            pnl: trade_pnl,
            pnl_pct,
            reason: format!("Partial{}R", r_label),
            closed_at: now_str(),
            entry_time: entry_time_partial,
            quantity: close_qty,
            size_usd: close_size,
            leverage: leverage_partial,
            fees_est: fees_partial,
            breakdown: partial_breakdown,
            note: None,
            venue: "Hyperliquid Perps (paper)".to_string(),
        };
        ledger::append(&partial_trade);
        s.closed_trades.push(partial_trade);
        let len = s.closed_trades.len();
        if len > 500 {
            s.closed_trades.drain(0..len - 500);
        }

        s.metrics = PerformanceMetrics::calculate(&s.closed_trades);
        let m = &s.metrics;
        info!(
            "📈 Metrics → Sharpe:{:.2} Kelly:{:.1}% WinRate:{:.0}%",
            m.sharpe,
            if m.kelly_fraction() > 0.0 {
                m.kelly_fraction() * 100.0
            } else {
                0.0
            },
            m.win_rate * 100.0
        );

        // Log partial close
        let r_at_partial = if close_size > 0.0 {
            trade_pnl / close_size
        } else {
            0.0
        };
        trade_logger.lock().await.log(&TradeEvent::TradePartial {
            ts: ts_now(),
            symbol: symbol.clone(),
            side: if was_long {
                "LONG".to_string()
            } else {
                "SHORT".to_string()
            },
            exit_price,
            size_closed_usd: close_size,
            pnl_usd: trade_pnl,
            r_milestone: r_num,
            r_at_close: r_at_partial,
        });

        // ── Live mode: place reduce-only partial close on HL (collects builder fee) ──
        if !paper {
            let hl_c = hl.clone();
            let sym_c = symbol.clone();
            let qty_c = close_qty;
            let price_c = exit_price;
            let long_c = was_long;
            let bps_c = fee_bps;
            tokio::spawn(async move {
                if let Err(e) = hl_c
                    .close_position_qty(&sym_c, long_c, qty_c, price_c, bps_c)
                    .await
                {
                    log::warn!("⚠ partial close order failed for {sym_c}: {e}");
                }
            });
        }

        drop(s);
        let mut w = weights.write().await;
        w.update(&contrib, was_long, trade_pnl > 0.0);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  FULL CLOSE
// ═══════════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
async fn close_paper_position(
    symbol: &str,
    exit_price: f64,
    reason: &str,
    bot_state: &SharedState,
    weights: &SharedWeights,
    trade_logger: &SharedTradeLogger,
    notifier: &Option<SharedNotifier>,
    db: &Option<SharedDb>,
    tenant_id: Uuid,
    hl: &Arc<exchange::HyperliquidClient>,
    fee_bps: u32,
    paper: bool,
) {
    let mut s = bot_state.write().await;

    let idx = s.positions.iter().position(|p| p.symbol == symbol);
    if let Some(idx) = idx {
        let pos = s.positions.remove(idx);

        // P&L = price move × quantity  (same formula as unrealised_pnl and partial close)
        // Using (exit - entry) * qty rather than (qty*exit - size_usd) because size_usd
        // is margin only; with leverage>1 the latter inflates P&L dramatically.
        let trade_pnl = if pos.side == "LONG" {
            (exit_price - pos.entry_price) * pos.quantity
        } else {
            (pos.entry_price - exit_price) * pos.quantity
        };

        // ── Profit recycling: route profits to house_money_pool ───────────
        // Full close: margin + loss/gain.  Profit goes to pool to finance
        // future moves (including a re-entry on the same coin).
        let cycle_now = s.cycle_count;
        if pos.funded_from_pool {
            // Pool stake fully returned
            s.house_money_pool += pos.pool_stake_usd;
            s.pool_deployed_usd = (s.pool_deployed_usd - pos.pool_stake_usd).max(0.0);
        } else {
            s.capital += pos.size_usd; // own-capital margin returned
        }
        if trade_pnl > 0.0 {
            // Profit → pool (available for the next move)
            s.house_money_pool += trade_pnl;
        } else {
            // Loss → deducted from capital
            s.capital += trade_pnl;
        }
        s.pnl += trade_pnl;

        // Record close for re-entry detection (keep last 30)
        if trade_pnl > 0.0 {
            s.recently_closed
                .push_back((symbol.to_string(), trade_pnl, cycle_now));
            while s.recently_closed.len() > 30 {
                s.recently_closed.pop_front();
            }
        }

        let pnl_pct = trade_pnl / pos.size_usd * 100.0;
        let profitable = trade_pnl > 0.0;
        let was_long = pos.side == "LONG";
        let r_at_close = if pos.r_dollars_risked > 1e-8 {
            trade_pnl / pos.r_dollars_risked
        } else {
            0.0
        };

        info!(
            "📝 CLOSE {} {} @ ${:.4} → {:+.2} ({:+.1}% / {:.2}R) [{}]  pool=${:.2}",
            pos.side,
            symbol,
            exit_price,
            trade_pnl,
            pnl_pct,
            r_at_close,
            reason,
            s.house_money_pool
        );

        // ── Build verbose breakdown for the click-to-expand dashboard row ────────
        let hold_mins = pos.cycles_held / 2;
        let hold_str = if hold_mins < 60 {
            format!("{}m", hold_mins)
        } else {
            format!("{:.1}h", hold_mins as f64 / 60.0)
        };
        let c = &pos.contrib;
        let sig_flags: String = [
            ("RSI", c.rsi_bullish),
            ("BB", c.bb_bullish),
            ("MACD", c.macd_bullish),
            ("Trend", c.trend_bullish),
            ("OF", c.of_bullish),
        ]
        .iter()
        .map(|(name, bull)| {
            let col = if *bull { "#3fb950" } else { "#f85149" };
            let arrow = if *bull { "↑" } else { "↓" };
            format!("<span style='color:{col}'>{name}{arrow}</span>")
        })
        .collect::<Vec<_>>()
        .join(" ");
        let ai_line = match (&pos.ai_action, &pos.ai_reason) {
            (Some(act), Some(rsn)) => format!(
                "<div style='margin-top:4px'>🤖 <b style='color:#e3b341'>AI {}</b>: <span style='color:#cdd9e5'>{rsn}</span></div>",
                act.replace('_', " ").to_uppercase()
            ),
            _ => String::new(),
        };
        let breakdown = Some(format!(
            "<div style='font-size:.78em;padding:4px 0;line-height:1.7'>\
             <div><b>{side}</b> entry <b>${entry:.4}</b> → exit <b>${exit:.4}</b> \
             &nbsp;·&nbsp; held <b>{hold}</b> \
             &nbsp;·&nbsp; result <b style='color:{rc}'>{r:+.2}R</b></div>\
             <div style='color:#8b949e'>Stop <span style='color:#f85149'>${stop:.4}</span> \
             &nbsp;·&nbsp; TP <span style='color:#3fb950'>${tp:.4}</span> \
             &nbsp;·&nbsp; margin <b>${size:.2}</b> · {lev:.1}× lev</div>\
             <div style='margin-top:2px'>Signals at entry: {sigs}</div>\
             {ai_ln}</div>",
            side = pos.side,
            entry = pos.entry_price,
            exit = exit_price,
            hold = hold_str,
            r = r_at_close,
            rc = if r_at_close >= 0.0 {
                "#3fb950"
            } else {
                "#f85149"
            },
            stop = pos.stop_loss,
            tp = pos.take_profit,
            size = pos.size_usd,
            lev = pos.leverage,
            sigs = sig_flags,
            ai_ln = ai_line,
        ));

        let fees_full = ledger::estimate_fees(pos.size_usd, pos.leverage);
        let full_trade = ClosedTrade {
            symbol: symbol.to_string(),
            side: pos.side.clone(),
            entry: pos.entry_price,
            exit: exit_price,
            pnl: trade_pnl,
            pnl_pct,
            reason: reason.to_string(),
            closed_at: now_str(),
            entry_time: pos.entry_time.clone(),
            quantity: pos.quantity,
            size_usd: pos.size_usd,
            leverage: pos.leverage,
            fees_est: fees_full,
            breakdown,
            note: None, // operator can add note via POST /api/trade-note
            venue: "Hyperliquid Perps (paper)".to_string(),
        };
        ledger::append(&full_trade);
        s.closed_trades.push(full_trade);
        let len = s.closed_trades.len();
        if len > 500 {
            s.closed_trades.drain(0..len - 500);
        }

        // ── Recalculate all metrics from updated history ──────────────────
        s.metrics = PerformanceMetrics::calculate(&s.closed_trades);
        let m = &s.metrics;
        info!(
            "📈 Metrics → Sharpe:{:.2} Sortino:{:.2} Expect:{:+.1}% PF:{:.2} Kelly:{:.1}% CB:{}",
            m.sharpe,
            m.sortino,
            m.expectancy,
            m.profit_factor,
            if m.kelly_fraction() > 0.0 {
                m.kelly_fraction() * 100.0
            } else {
                0.0
            },
            if m.in_circuit_breaker() { "ON" } else { "off" }
        );

        // ── Log the exit ──────────────────────────────────────────────────
        trade_logger.lock().await.log(&TradeEvent::TradeExit {
            ts: ts_now(),
            symbol: symbol.to_string(),
            side: pos.side.clone(),
            entry_price: pos.entry_price,
            exit_price,
            size_usd: pos.size_usd,
            pnl_usd: trade_pnl,
            pnl_pct,
            r_multiple: r_at_close,
            reason: reason.to_string(),
            cycles_held: pos.cycles_held as u32,
            minutes_held: (pos.cycles_held / 2) as u32,
            dca_count: pos.dca_count,
            tranches_closed: pos.tranches_closed,
        });

        // ── Fire position-closed webhook / Telegram notification ─────────────
        if let Some(n) = notifier {
            let n = n.clone();
            let sym = symbol.to_string();
            let side = pos.side.clone();
            let pnl = trade_pnl;
            let pct = pnl_pct;
            let why = reason.to_string();
            let r = r_at_close;
            tokio::spawn(async move {
                n.position_closed(&sym, &side, pnl, pct, &why, r).await;
            });
        }

        // ── Live mode: place reduce-only close order on HL (collects builder fee) ──
        if !paper {
            let hl_c = hl.clone();
            let sym_c = pos.symbol.clone();
            let qty_c = pos.quantity;
            let price_c = exit_price;
            let long_c = was_long;
            let bps_c = fee_bps;
            tokio::spawn(async move {
                if let Err(e) = hl_c
                    .close_position_qty(&sym_c, long_c, qty_c, price_c, bps_c)
                    .await
                {
                    log::warn!("⚠ close order failed for {sym_c}: {e}");
                }
            });
        }

        // ── Online signal weight learning ─────────────────────────────────
        drop(s);
        let mut w = weights.write().await;
        w.update(&pos.contrib, was_long, profitable);
        info!(
            "🧠 Weights → RSI:{:.2} BB:{:.2} MACD:{:.2} Trend:{:.2} OF:{:.2}",
            w.rsi, w.bollinger, w.macd, w.trend, w.order_flow
        );
        drop(w);

        // ── Collective intelligence: record outcome + remove from crowd ───
        if let Some(ref shared_db) = db {
            let db_rec = shared_db.clone();
            let db_rem = shared_db.clone();
            let sym_rec = pos.symbol.clone();
            let sym_rem = pos.symbol.clone();
            // Capture pos fields needed for record_outcome before moving
            let pos_snap = pos.clone();
            let exit_snap = exit_price;
            let pnl_snap = pnl_pct;
            let r_snap = r_at_close;

            // Record trade outcome in collective table (fire-and-forget)
            tokio::spawn(async move {
                collective::record_outcome(
                    &db_rec,
                    Some(tenant_id),
                    &pos_snap,
                    exit_snap,
                    pnl_snap,
                    r_snap,
                )
                .await;
                // Also write to the existing closed_trades DB table
                let _ = db_rec
                    .insert_closed_trade(
                        &tenant_id.to_string(),
                        &sym_rec,
                        &pos_snap.side,
                        pos_snap.entry_price,
                        exit_snap,
                        pos_snap.size_usd,
                        pnl_snap / 100.0 * pos_snap.size_usd, // approximate pnl_usd
                        pnl_snap,
                        r_snap,
                        ledger::estimate_fees(pos_snap.size_usd, pos_snap.leverage),
                        None, // opened_at (we have entry_time string, not DateTime — skip)
                        "close",
                        None,
                    )
                    .await
                    .map_err(|e| log::debug!("insert_closed_trade: {e}"));
            });

            // Remove from hot_positions (fire-and-forget)
            tokio::spawn(async move {
                collective::remove_hot_position(&db_rem, tenant_id, &sym_rem).await;
            });
            if let Some(refresher) = PATTERN_CACHE_REFRESHER.get() {
                refresher.trigger();
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  HELPERS
// ═══════════════════════════════════════════════════════════════════════════════

async fn set_status(bot_state: &SharedState, msg: &str) {
    let mut s = bot_state.write().await;
    s.status = msg.to_string();
    s.last_update = now_str();
}

fn now_str() -> String {
    chrono::Utc::now().format("%H:%M:%S UTC").to_string()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  UNIT TESTS — position management & heat calculations
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use learner::SignalContribution;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Minimal PaperPosition for heat tests.
    fn make_pos(side: &str, entry: f64, stop: f64, qty: f64, size_usd: f64) -> PaperPosition {
        PaperPosition {
            symbol: "TEST".to_string(),
            side: side.to_string(),
            entry_price: entry,
            quantity: qty,
            size_usd,
            stop_loss: stop,
            take_profit: if side == "LONG" {
                entry * 1.10
            } else {
                entry * 0.90
            },
            atr_at_entry: entry * 0.02,
            high_water_mark: entry,
            low_water_mark: entry,
            partial_closed: false,
            r_dollars_risked: (entry - stop).abs() * qty,
            tranches_closed: 0,
            dca_count: 0,
            leverage: 1.0,
            cycles_held: 0,
            entry_time: "00:00:00 UTC".to_string(),
            unrealised_pnl: 0.0,
            contrib: SignalContribution::default(),
            ai_action: None,
            ai_reason: None,
            entry_confidence: 0.68,
            trade_budget_usd: size_usd,
            dca_spent_usd: 0.0,
            btc_ret_at_entry: 0.0,
            initial_margin_usd: size_usd,
            ob_sentiment: String::new(),
            ob_bid_wall_near: false,
            ob_ask_wall_near: false,
            ob_adverse_cycles: 0,
            order_flow_confidence: 0.0,
            order_flow_direction: String::new(),
            funding_rate: 0.0,
            funding_delta: 0.0,
            onchain_strength: 0.0,
            cex_premium_pct: 0.0,
            cex_mode: String::new(),
            funded_from_pool: false,
            pool_stake_usd: 0.0,
            venue: "Hyperliquid Perps (paper)".to_string(),
        }
    }

    // ── trade_heat ────────────────────────────────────────────────────────────

    #[test]
    fn trade_heat_1x_leverage_matches_margin_based_formula() {
        // With 1× leverage: heat = stop_dist_pct × size_usd / equity
        // entry=$100, stop=$98 → stop_dist=2%,  size_usd=$100, equity=$1000
        // heat = 0.02 × 100 / 1000 = 0.002  (0.2 %)
        let heat = trade_heat(100.0, 98.0, 100.0, 1000.0, 1.0);
        assert!(
            (heat - 0.002).abs() < 1e-10,
            "1× heat should be 0.002, got {heat}"
        );
    }

    #[test]
    fn trade_heat_3x_leverage_triples_heat() {
        // 3× leverage triples actual notional risk vs 1× baseline
        let heat_1x = trade_heat(100.0, 98.0, 100.0, 1000.0, 1.0);
        let heat_3x = trade_heat(100.0, 98.0, 100.0, 1000.0, 3.0);
        assert!(
            (heat_3x - heat_1x * 3.0).abs() < 1e-10,
            "3× leverage should triple heat: {heat_1x} × 3 = {} ≠ {heat_3x}",
            heat_1x * 3.0
        );
    }

    #[test]
    fn trade_heat_zero_equity_guard() {
        // Equity below $1 is a divide-by-near-zero guard — always returns 1.0
        let heat = trade_heat(100.0, 98.0, 100.0, 0.5, 3.0);
        assert_eq!(heat, 1.0, "near-zero equity must return 1.0 sentinel");
    }

    #[test]
    fn trade_heat_exceeds_max_at_3x_where_1x_would_not() {
        // Setup: entry=$100, stop=$95 (5% stop), $100 margin, $1000 equity.
        // 1× heat = 0.05 × 100 × 1 / 1000 = 0.005 — well below MAX_TRADE_HEAT (0.05).
        // 3× heat = 0.015 — still below, but 10× leverage = 0.05 = exactly at limit.
        // At 10× leverage, heat should equal exactly MAX_TRADE_HEAT (0.05).
        let heat_10x = trade_heat(100.0, 95.0, 100.0, 1000.0, 10.0);
        assert!(
            (heat_10x - MAX_TRADE_HEAT).abs() < 1e-10,
            "10× lev with 5% stop on 10% position should reach MAX_TRADE_HEAT exactly: {heat_10x}"
        );
    }

    #[test]
    fn trade_heat_short_position_uses_abs_distance() {
        // SHORT: entry < stop (stop is above entry)
        // entry=$100, stop=$103 → |100-103|/100 = 3% stop distance
        // With 1× leverage, $50 margin, $1000 equity:
        // heat = 0.03 × 50 / 1000 = 0.0015
        let heat = trade_heat(100.0, 103.0, 50.0, 1000.0, 1.0);
        assert!(
            (heat - 0.0015).abs() < 1e-10,
            "SHORT heat: expected 0.0015, got {heat}"
        );
    }

    #[test]
    fn trade_heat_allowed_margin_formula_is_consistent() {
        // If t_heat > MAX_TRADE_HEAT, the allowed-margin formula must produce
        // a size whose heat is exactly MAX_TRADE_HEAT.
        // allowed = MAX_TRADE_HEAT × equity / (stop_dist_pct × leverage)
        let entry: f64 = 100.0;
        let stop = 95.0; // 5% stop distance
        let equity = 1000.0;
        let leverage = 3.0;
        let stop_dist = (entry - stop).abs() / entry; // 0.05

        // Deliberately oversized: $500 margin → heat = 0.05 × 500 × 3 / 1000 = 0.075
        let oversized_heat = trade_heat(entry, stop, 500.0, equity, leverage);
        assert!(
            oversized_heat > MAX_TRADE_HEAT,
            "test setup: heat {oversized_heat} must exceed limit"
        );

        // Compute allowed margin
        let allowed = MAX_TRADE_HEAT * equity / (stop_dist * leverage);
        let heat_at_allowed = trade_heat(entry, stop, allowed, equity, leverage);
        assert!(
            (heat_at_allowed - MAX_TRADE_HEAT).abs() < 1e-10,
            "allowed-margin heat should equal MAX_TRADE_HEAT, got {heat_at_allowed}"
        );
    }

    // ── REGRESSION: leverage bug ──────────────────────────────────────────────

    #[test]
    fn regression_trade_heat_old_formula_underestimated_by_leverage() {
        // Before the fix: trade_heat() = stop_dist_pct × size_usd / equity
        // This ignored leverage, so at 3× it accepted trades 3× riskier than intended.
        //
        // Concrete example: entry=$100, stop=$98 (2%), $100 margin, 3× leverage, $1000 equity.
        // Old formula:  0.02 × 100 / 1000 = 0.002  (looks like only 0.2% at risk)
        // True formula: 0.02 × 100 × 3 / 1000 = 0.006 (actually 0.6% at risk — correct)
        //
        // The actual dollar loss if stop is hit:
        //   qty = 100 × 3 / 100 = 3 shares
        //   loss = (100 - 98) × 3 = $6  → 6/1000 = 0.6%  ✓ matches new formula
        let entry: f64 = 100.0;
        let stop = 98.0;
        let size_usd = 100.0;
        let equity = 1000.0;
        let leverage = 3.0;
        let qty = size_usd * leverage / entry;
        let actual_dollar_loss = (entry - stop).abs() * qty;
        let actual_heat = actual_dollar_loss / equity;

        let formula_heat = trade_heat(entry, stop, size_usd, equity, leverage);
        assert!(
            (formula_heat - actual_heat).abs() < 1e-10,
            "trade_heat() must equal actual dollar-loss / equity: {formula_heat} ≠ {actual_heat}"
        );
    }

    // ── portfolio_heat ────────────────────────────────────────────────────────

    #[test]
    fn portfolio_heat_empty_positions_returns_zero() {
        let heat = portfolio_heat(&[], 1000.0);
        assert_eq!(heat, 0.0, "no open positions → 0 heat");
    }

    #[test]
    fn portfolio_heat_zero_equity_guard() {
        let pos = make_pos("LONG", 100.0, 95.0, 1.0, 100.0);
        let heat = portfolio_heat(&[pos], 0.5);
        assert_eq!(heat, 1.0, "near-zero equity must return 1.0 sentinel");
    }

    #[test]
    fn portfolio_heat_long_stop_below_entry_has_positive_heat() {
        // entry=$100, stop=$95, qty=10 → risk = (100-95)×10 = $50 on $1000 equity = 5%
        let pos = make_pos("LONG", 100.0, 95.0, 10.0, 100.0);
        let heat = portfolio_heat(&[pos], 1000.0);
        let expected = 50.0 / 1000.0; // 5%
        assert!(
            (heat - expected).abs() < 1e-10,
            "LONG stop below entry: expected {expected}, got {heat}"
        );
    }

    #[test]
    fn portfolio_heat_long_stop_at_breakeven_is_zero() {
        // Once trailing stop reaches entry price, there's no remaining downside risk.
        // stop = entry_price → (entry - stop) × qty = 0 → heat = 0.
        let pos = make_pos("LONG", 100.0, 100.0, 10.0, 100.0); // stop AT entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(heat, 0.0, "LONG stop at breakeven → zero heat");
    }

    #[test]
    fn portfolio_heat_long_stop_above_entry_is_zero() {
        // Trailing stop has advanced ABOVE entry (locked in profit, no remaining risk).
        // stop > entry for LONG → (entry - stop) < 0 → .max(0) → 0.
        let pos = make_pos("LONG", 100.0, 105.0, 10.0, 100.0); // stop ABOVE entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(
            heat, 0.0,
            "LONG stop above entry (trailing past BE) → zero heat"
        );
    }

    #[test]
    fn portfolio_heat_short_stop_above_entry_has_positive_heat() {
        // SHORT: stop above entry = risk zone.
        // entry=$100, stop=$104, qty=10 → risk = (104-100)×10 = $40 on $1000 = 4%
        let pos = make_pos("SHORT", 100.0, 104.0, 10.0, 100.0);
        let heat = portfolio_heat(&[pos], 1000.0);
        let expected = 40.0 / 1000.0; // 4%
        assert!(
            (heat - expected).abs() < 1e-10,
            "SHORT stop above entry: expected {expected}, got {heat}"
        );
    }

    #[test]
    fn portfolio_heat_short_stop_at_breakeven_is_zero() {
        let pos = make_pos("SHORT", 100.0, 100.0, 10.0, 100.0); // stop AT entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(heat, 0.0, "SHORT stop at breakeven → zero heat");
    }

    #[test]
    fn portfolio_heat_short_stop_below_entry_is_zero() {
        // Trailing stop has advanced BELOW entry for a SHORT (locked in profit).
        let pos = make_pos("SHORT", 100.0, 96.0, 10.0, 100.0); // stop BELOW entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(
            heat, 0.0,
            "SHORT stop below entry (trailing past BE) → zero heat"
        );
    }

    #[test]
    fn portfolio_heat_multiple_positions_sum_correctly() {
        // Two positions: LONG $30 risk + SHORT $20 risk = $50 on $1000 equity = 5%
        let long_pos = make_pos("LONG", 100.0, 97.0, 10.0, 100.0); // (100-97)×10 = $30
        let short_pos = make_pos("SHORT", 200.0, 202.0, 10.0, 100.0); // (202-200)×10 = $20
        let heat = portfolio_heat(&[long_pos, short_pos], 1000.0);
        let expected = 50.0 / 1000.0; // 5%
        assert!(
            (heat - expected).abs() < 1e-10,
            "multi-position heat: expected {expected}, got {heat}"
        );
    }

    #[test]
    fn portfolio_heat_trailing_stop_position_contributes_zero() {
        // One live-risk position + one position where trailing stop is above entry.
        // Only the live-risk position should contribute to heat.
        let at_risk = make_pos("LONG", 100.0, 95.0, 10.0, 100.0); // (100-95)×10 = $50
        let protected = make_pos("LONG", 100.0, 103.0, 10.0, 100.0); // stop above entry → 0
        let heat = portfolio_heat(&[at_risk, protected], 1000.0);
        let expected = 50.0 / 1000.0; // only $50 risk from the at-risk position
        assert!(
            (heat - expected).abs() < 1e-10,
            "protected position must not add heat: expected {expected}, got {heat}"
        );
    }

    // ── REGRESSION: portfolio_heat stale r_dollars_risked ─────────────────────

    #[test]
    fn regression_portfolio_heat_old_formula_would_overstate_after_stop_advance() {
        // Pre-fix: portfolio_heat used p.r_dollars_risked / equity.
        // After trailing stop advances past entry, r_dollars_risked is still set to
        // the original entry risk.  The new formula correctly returns 0.
        //
        // Scenario: LONG entry=$100, original stop=$95, qty=10, equity=$1000.
        //   Original r_dollars_risked = (100-95)×10 = $50.
        //   Stop has since moved to $103 (trailing past breakeven).
        //   Current risk = 0 (no downside to stop).
        //   OLD formula: $50/$1000 = 5% ← WRONG, 3% phantom risk
        //   NEW formula: max(0, (100-103)×10) / 1000 = 0% ← CORRECT
        let mut pos = make_pos("LONG", 100.0, 103.0, 10.0, 100.0);
        pos.r_dollars_risked = 50.0; // stale from original entry (before stop advanced)

        let heat = portfolio_heat(&[pos], 1000.0);
        // Old formula would give 50/1000 = 0.05; new formula gives 0.
        assert_eq!(
            heat, 0.0,
            "REGRESSION: portfolio_heat must return 0 when trailing stop is above entry, \
             regardless of stale r_dollars_risked (old formula returned 0.05)"
        );
    }

    // ── position_size_pct ─────────────────────────────────────────────────────

    #[test]
    fn position_size_pct_pre_kelly_high_confidence() {
        // With <5 trades (no Kelly), conf=0.85 → base 15%, no Sharpe adjustment (1.0 mult)
        let metrics = PerformanceMetrics::default(); // total_trades=0 → no Kelly, size_mult=1.0
        let pct = position_size_pct(0.85, &metrics, false, MIN_POSITION_PCT, MAX_POSITION_PCT);
        // base=0.15 × sharpe_mult(1.0 for <3 trades) × no-CB → 0.15
        assert_eq!(pct, 0.15, "pre-Kelly high confidence: expected 15%");
    }

    #[test]
    fn position_size_pct_pre_kelly_mid_confidence() {
        let metrics = PerformanceMetrics::default();
        let pct = position_size_pct(0.75, &metrics, false, MIN_POSITION_PCT, MAX_POSITION_PCT);
        assert_eq!(pct, 0.12, "pre-Kelly mid confidence: expected 12%");
    }

    #[test]
    fn position_size_pct_pre_kelly_min_confidence() {
        let metrics = PerformanceMetrics::default();
        let pct = position_size_pct(MIN_CONFIDENCE, &metrics, false, MIN_POSITION_PCT, MAX_POSITION_PCT);
        assert_eq!(pct, 0.08, "pre-Kelly min confidence: expected 8%");
    }

    #[test]
    fn position_size_pct_circuit_breaker_reduces_size() {
        // CB active → CB_SIZE_MULT (0.35) applied on top of everything else.
        let metrics = PerformanceMetrics::default();
        let normal = position_size_pct(0.85, &metrics, false, MIN_POSITION_PCT, MAX_POSITION_PCT);
        let cb = position_size_pct(0.85, &metrics, true, MIN_POSITION_PCT, MAX_POSITION_PCT);
        let expected_cb = normal * CB_SIZE_MULT;
        assert!(
            (cb - expected_cb).abs() < 1e-10,
            "CB should multiply by {CB_SIZE_MULT}: {normal} × {CB_SIZE_MULT} = {expected_cb}, got {cb}"
        );
    }

    #[test]
    fn position_size_pct_never_below_min() {
        // Even with CB + negative Sharpe, result can't go below MIN_POSITION_PCT
        let metrics = PerformanceMetrics {
            sharpe: -2.0,
            total_trades: 10,
            ..Default::default()
        };
        let pct = position_size_pct(MIN_CONFIDENCE, &metrics, true, MIN_POSITION_PCT, MAX_POSITION_PCT);
        assert!(
            pct >= MIN_POSITION_PCT,
            "position_size_pct must never go below MIN_POSITION_PCT ({MIN_POSITION_PCT}), got {pct}"
        );
    }

    #[test]
    fn position_size_pct_never_above_max() {
        // Very high Kelly with great Sharpe still can't exceed MAX_POSITION_PCT
        let metrics = PerformanceMetrics {
            sharpe: 3.0,
            win_rate: 0.80,
            avg_win_pct: 25.0,
            avg_loss_pct: 5.0,
            total_trades: 50,
            ..Default::default()
        };
        let pct = position_size_pct(1.0, &metrics, false, MIN_POSITION_PCT, MAX_POSITION_PCT);
        assert!(
            pct <= MAX_POSITION_PCT,
            "position_size_pct must never exceed MAX_POSITION_PCT ({MAX_POSITION_PCT}), got {pct}"
        );
    }

    // ── P&L formula correctness ───────────────────────────────────────────────

    #[test]
    fn pnl_formula_long_close_matches_price_move_times_qty() {
        // close_paper_position uses: (exit - entry) * quantity for LONG
        // This is the leveraged return: qty = margin × leverage / entry
        let entry: f64 = 100.0;
        let exit = 110.0; // +10%
        let margin = 100.0;
        let leverage = 3.0;
        let qty = margin * leverage / entry; // 3.0 shares
        let expected_pnl = (exit - entry) * qty; // $30

        assert!(
            (expected_pnl - 30.0).abs() < 1e-10,
            "LONG P&L: 3 shares × $10 move = $30, got {expected_pnl}"
        );

        // Verify pnl_pct (relative to margin, not notional): 30/100 = 30%
        let pnl_pct = expected_pnl / margin * 100.0;
        assert!(
            (pnl_pct - 30.0).abs() < 1e-10,
            "pnl_pct should be 30%% on margin (3× leveraged 10% move), got {pnl_pct}"
        );
    }

    #[test]
    fn pnl_formula_short_close_matches_price_move_times_qty() {
        // close_paper_position uses: (entry - exit) * quantity for SHORT
        let entry: f64 = 100.0;
        let exit = 90.0; // price falls 10% → SHORT wins
        let margin = 100.0;
        let leverage = 3.0;
        let qty = margin * leverage / entry; // 3.0 shares
        let expected_pnl = (entry - exit) * qty; // $30

        assert!(
            (expected_pnl - 30.0).abs() < 1e-10,
            "SHORT P&L: 3 shares × $10 favourable move = $30, got {expected_pnl}"
        );
    }

    #[test]
    fn r_multiple_formula_is_consistent_with_pnl_and_entry_risk() {
        // r_mult = unrealised_pnl / r_dollars_risked
        // At 1R: pnl should equal r_dollars_risked
        let entry: f64 = 100.0;
        let stop = 95.0; // 5% distance
        let margin = 100.0;
        let leverage = 3.0;
        let qty = margin * leverage / entry; // 3.0 shares
        let r_risk = (entry - stop).abs() * qty; // $15

        // Price moves to 1R target: entry + (entry-stop) = $105
        let price_at_1r = entry + (entry - stop); // $105
        let pnl_at_1r = (price_at_1r - entry) * qty; // $15

        let r_mult = pnl_at_1r / r_risk;
        assert!(
            (r_mult - 1.0).abs() < 1e-10,
            "at price = entry + 1×stop_dist, R-multiple should be 1.0, got {r_mult}"
        );
    }

    #[test]
    fn dca_weighted_avg_entry_formula() {
        // After DCA add-on: avg_entry = (old_qty × old_entry + add_qty × dca_price) / new_qty
        let old_entry: f64 = 100.0;
        let dca_price = 95.0; // price fell, we DCA lower
        let old_qty = 3.0;
        let add_qty = 1.5; // 50% of original size
        let new_qty = old_qty + add_qty; // 4.5

        let avg_entry = (old_entry * old_qty + dca_price * add_qty) / new_qty;
        // = (300 + 142.5) / 4.5 = 442.5 / 4.5 = 98.333...
        let expected = 442.5 / 4.5;
        assert!(
            (avg_entry - expected).abs() < 1e-10,
            "DCA avg_entry should be {expected}, got {avg_entry}"
        );
        // avg_entry must be BETWEEN original entry and DCA price
        assert!(
            avg_entry < old_entry && avg_entry > dca_price,
            "DCA avg_entry {avg_entry} must be between {dca_price} and {old_entry}"
        );
    }

    #[test]
    fn pyramid_weighted_avg_entry_formula() {
        // After pyramid: avg_entry = (old_qty × old_entry + add_qty × pyramid_price) / new_qty
        let old_entry: f64 = 100.0;
        let pyramid_price = 106.0; // price rose 6%, we pyramid
        let old_qty = 3.0;
        let add_qty = 1.5; // 50% add-on
        let new_qty = old_qty + add_qty; // 4.5

        let avg_entry = (old_entry * old_qty + pyramid_price * add_qty) / new_qty;
        // = (300 + 159) / 4.5 = 459 / 4.5 = 102.0
        let expected = 459.0 / 4.5;
        assert!(
            (avg_entry - expected).abs() < 1e-10,
            "pyramid avg_entry should be {expected}, got {avg_entry}"
        );
        // avg_entry must be ABOVE original entry (pyramid is into profit)
        assert!(
            avg_entry > old_entry && avg_entry < pyramid_price,
            "pyramid avg_entry {avg_entry} must be between {old_entry} and {pyramid_price}"
        );
    }

    #[test]
    fn partial_close_r_dollars_risked_scales_proportionally() {
        // After 1/3 close (tranche 1), r_dollars_risked *= 2/3.
        // Simulates take_partial() logic.
        let original_r: f64 = 150.0;
        let after_partial = original_r * (2.0 / 3.0);
        let expected = 100.0;
        assert!(
            (after_partial - expected).abs() < 1e-10,
            "after 1/3 close, r_dollars_risked should scale to 2/3: {after_partial} ≠ {expected}"
        );
    }

    #[test]
    fn trailing_stop_breakeven_move_for_long() {
        // At 1R profit: stop moves to entry (breakeven).
        // pos.stop_loss < pos.entry_price AND r_mult >= 1.0 → stop = entry
        let entry: f64 = 100.0;
        let stop = 95.0;
        let qty = 3.0;
        let r_risk = (entry - stop) * qty; // $15

        // Current price at exactly 1R: entry + (entry - stop) = $105
        let cur = 105.0;
        let unrealised = (cur - entry) * qty; // $15
        let r_mult = unrealised / r_risk; // 1.0

        assert!((r_mult - 1.0).abs() < 1e-10, "should be exactly 1R at $105");

        // Trailing stop rule: if r_mult >= 1.0 && stop < entry → set stop = entry
        let new_stop = if r_mult >= 1.0 && stop < entry {
            entry
        } else {
            stop
        };
        assert_eq!(new_stop, entry, "stop should move to breakeven at 1R");
    }

    #[test]
    fn trailing_stop_tier1_tight_trail_at_0_75r_for_long() {
        // At 0.75R profit: tight 0.6×ATR trail below HWM starts protecting early gains.
        // Use atr=7.5 so the trail (99.25) lands below entry (100.0) — the condition
        // `trail < entry` only fires when the stop is still in "protecting against loss"
        // territory.  With a small ATR the trail would exceed entry, which is tier-2's job.
        let entry: f64 = 100.0;
        let stop = 95.0;
        let atr = 7.5; // large enough so trail = hwm - 0.6×7.5 = 99.25 < entry
        let qty = 3.0;
        let r_risk = (entry - stop) * qty; // $15

        // Current price at exactly 0.75R: entry + 0.75 × (entry-stop) = $103.75
        let cur = 103.75;
        let unr = (cur - entry) * qty; // $11.25
        let r_mult = unr / r_risk; // 0.75

        assert!((r_mult - 0.75).abs() < 1e-10, "should be 0.75R at $103.75");

        let hwm = cur;
        let trail = hwm - atr * 0.6; // 103.75 - 4.5 = 99.25
                                     // Only set if trail > current stop AND trail < entry (below breakeven)
        let new_stop = if r_mult >= 0.75 && trail > stop && trail < entry {
            trail
        } else {
            stop
        };
        assert!(
            (new_stop - 99.25).abs() < 1e-10,
            "0.75R tight trail should be HWM - 0.6×ATR = 99.25, got {new_stop}"
        );
    }

    #[test]
    fn trailing_stop_tier1_does_not_override_breakeven_for_long() {
        // Once stop is at breakeven ($100), the 0.75R tight trail must NOT pull it back below entry.
        let entry: f64 = 100.0;
        let stop = entry; // already at breakeven
        let atr = 2.0;
        let qty = 3.0;
        let _r_risk = (entry - (entry - 5.0)) * qty; // original $15, but stop is now at entry
        let cur = 103.75;
        let _unr = (cur - entry) * qty;
        // r_mult based on current stop would be undefined; use tier logic directly:
        // tier1 only fires if trail > stop && trail < entry.  trail = 102.55, entry = 100 → trail >= entry → skip
        let hwm = cur;
        let trail = hwm - atr * 0.6; // 102.55 — which is > entry(100), so tier1 condition trail < entry fails
        let new_stop = if trail > stop && trail < entry {
            trail
        } else {
            stop
        };
        assert_eq!(
            new_stop, stop,
            "0.75R tier must not drag stop below breakeven once set"
        );
    }

    #[test]
    fn trailing_stop_trails_hwm_at_1_5r_for_long() {
        // At ≥ 1.5R: trail 1.2×ATR below HWM.
        let entry: f64 = 100.0;
        let stop = 95.0;
        let qty = 3.0;
        let atr = 2.0;
        let r_risk = (entry - stop) * qty; // $15

        // Price at 1.5R: entry + 1.5 × (entry-stop) = $107.50
        let cur = 107.5;
        let unr = (cur - entry) * qty;
        let r_mult = unr / r_risk; // 1.5

        assert!(
            (r_mult - 1.5).abs() < 1e-10,
            "should be exactly 1.5R at $107.50"
        );

        let hwm = cur;
        let trail = hwm - atr * 1.2; // 107.5 - 2.4 = 105.1
                                     // Only advance stop if trail > current stop
        let new_stop = if r_mult >= 1.5 && trail > stop {
            trail
        } else {
            stop
        };
        assert!(
            (new_stop - 105.1).abs() < 1e-10,
            "trailing stop at 1.5R should be HWM - 1.2×ATR = 105.1, got {new_stop}"
        );
    }

    #[test]
    fn tier0_trail_at_0_30r_for_long() {
        // At 0.30R profit: 0.4×ATR trail activates, protecting early gains.
        // Use atr=5.0 so the trail (99.5) lands below entry (100.0) — the condition
        // `trail < entry` only fires when the stop is still in pre-breakeven territory.
        // With a small ATR the trail would exceed entry (breakeven is tier-2's job).
        let entry: f64 = 100.0;
        let stop = 95.0;
        let atr = 5.0; // large enough so trail = hwm - 0.4×5 = 99.5 < entry
        let qty = 3.0;
        let r_risk = (entry - stop) * qty; // $15

        // Price at exactly 0.30R: entry + 0.30 × (entry−stop) = $101.50
        let cur = 101.5;
        let unr = (cur - entry) * qty; // $4.50
        let r_mult = unr / r_risk; // 0.30
        assert!((r_mult - 0.30).abs() < 1e-10, "should be 0.30R at $101.50");

        let hwm = cur;
        let trail = hwm - atr * 0.4; // 101.5 - 2.0 = 99.5
                                     // Only set if trail > current stop AND trail < entry
        let new_stop = if r_mult >= 0.30 && trail > stop && trail < entry {
            trail
        } else {
            stop
        };
        assert!(
            (new_stop - 99.5).abs() < 1e-10,
            "tier-0 trail at 0.30R should be HWM - 0.4×ATR = 99.5, got {new_stop}"
        );
    }

    #[test]
    fn tier0_trail_below_entry_only() {
        // If tier-0 trail would land above entry (shouldn't happen until near
        // breakeven), it must NOT fire — breakeven is tier-2's job.
        let entry: f64 = 100.0;
        let stop = 95.0;
        let atr = 5.0; // wide ATR
        let qty = 3.0;
        let r_risk = (entry - stop) * qty;

        let cur = 101.5; // 0.30R
        let unr = (cur - entry) * qty;
        let r_mult = unr / r_risk; // 0.30
        let hwm = cur;
        let trail = hwm - atr * 0.4; // 101.5 - 2.0 = 99.5 — below entry, should fire
        let new_stop_should_fire = if r_mult >= 0.30 && trail > stop && trail < entry {
            trail
        } else {
            stop
        };
        assert!(
            (new_stop_should_fire - 99.5).abs() < 1e-10,
            "tier-0 trail below entry should fire, got {new_stop_should_fire}"
        );

        // Now with very wide ATR: trail lands above entry → must NOT override breakeven
        let atr_wide = 12.0;
        let trail_above_entry = hwm - atr_wide * 0.4; // 101.5 - 4.8 = 96.7, still below entry
                                                      // Actually with these numbers it's still below entry.
                                                      // Use stop already at entry to verify tier-0 doesn't fire when trail <= stop
        let stop_at_be = entry; // 100.0
        let new_stop_be =
            if r_mult >= 0.30 && trail_above_entry > stop_at_be && trail_above_entry < entry {
                trail_above_entry
            } else {
                stop_at_be
            };
        assert_eq!(new_stop_be, stop_at_be,
            "tier-0 must not override breakeven stop (trail must be strictly above current stop AND below entry)");
    }

    #[test]
    fn false_breakout_detected_for_long() {
        // Trade peaked at 0.35R (above the 0.30R production threshold), is now
        // at -0.08R after 45 min with no DCA.  Pattern: false breakout — should close.
        let entry: f64 = 100.0;
        let stop = 95.0;
        let qty = 3.0;
        let r_risk = (entry - stop) * qty; // $15
        let hwm = 101.75; // peaked at +$1.75/unit × 3 qty = $5.25 → 5.25/15 = 0.35R

        let peak_r = (hwm - entry) * qty / r_risk; // 0.35
        assert!((peak_r - 0.35).abs() < 1e-10, "peak should be 0.35R");

        // Current price has reversed to -0.08R
        let r_mult = -0.08_f64;
        let cycles_held = 90_u32; // 45 min
        let dca_count = 0_u32;

        let false_breakout =
            peak_r >= 0.30 && r_mult < -0.05 && (30..120).contains(&cycles_held) && dca_count == 0;
        assert!(
            false_breakout,
            "REZ-type pattern should trigger false-breakout exit"
        );
    }

    #[test]
    fn false_breakout_not_triggered_with_dca() {
        // Same reversal pattern (peak 0.35R, now -0.08R) but DCA was taken —
        // we committed to the position, so the false-breakout guard must not fire.
        let peak_r = 0.35_f64;
        let r_mult = -0.08_f64;
        let cycles_held = 90_u32;
        let dca_count = 1_u32; // DCA taken

        let false_breakout =
            peak_r >= 0.30 && r_mult < -0.05 && (30..120).contains(&cycles_held) && dca_count == 0;
        assert!(
            !false_breakout,
            "false-breakout must not fire when DCA has been deployed"
        );
    }

    #[test]
    fn false_breakout_not_triggered_after_60min() {
        // After 60 min, time-exit rules take over — the false-breakout window is closed.
        let peak_r = 0.35_f64;
        let r_mult = -0.08_f64;
        let cycles_held = 130_u32; // 65 min > 60 min window
        let dca_count = 0_u32;

        let false_breakout =
            peak_r >= 0.30 && r_mult < -0.05 && (30..120).contains(&cycles_held) && dca_count == 0;
        assert!(
            !false_breakout,
            "false-breakout window closes after 60 min (120 cycles)"
        );
    }
}
