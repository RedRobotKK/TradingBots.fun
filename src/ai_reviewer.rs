//! Claude AI position reviewer.
//!
//! Every 10 cycles (~5 minutes) this module sends the current open positions
//! and portfolio metrics to the Claude API.  Claude acts as a senior quant
//! risk manager and returns structured scaling recommendations for each
//! position.
//!
//! ## Actions Claude can recommend
//!
//! | Action       | Effect                                           | Guard                          |
//! |-------------|--------------------------------------------------|--------------------------------|
//! | `scale_up`  | Increase position notional by `factor` (1.2–2.0×)| Only when R > 0.5 & held < 4 h|
//! | `hold`      | No change — let stop/TP strategy play out        | Default                        |
//! | `scale_down`| Reduce position to `factor` fraction (0.25–0.75×)| Min 25% of position must remain|
//! | `close_now` | Full close                                       | Only when R < −0.4R            |
//!
//! The bot executes recommendations within hard guardrails — Claude cannot
//! override the circuit breaker, heat limits, or stop-loss system.

use anyhow::{anyhow, Result};
use log::{info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::web_dashboard::PaperPosition;
use crate::metrics::PerformanceMetrics;

// ─────────────────────────────── Claude API types ────────────────────────────

/// Reusable Claude API request body.
/// `pub` so `daily_analyst` can build requests with a different model/prompt.
#[derive(Debug, Serialize)]
pub struct ClaudeRequest {
    pub model:      String,
    pub max_tokens: u32,
    pub system:     String,
    pub messages:   Vec<ClaudeMessage>,
}

#[derive(Debug, Serialize)]
pub struct ClaudeMessage {
    pub role:    String,
    pub content: String,
}

/// Convenience constructor — single user-turn request.
pub fn build_claude_request(
    model:      &str,
    max_tokens: u32,
    system:     &str,
    user_msg:   &str,
) -> ClaudeRequest {
    ClaudeRequest {
        model:      model.to_string(),
        max_tokens,
        system:     system.to_string(),
        messages:   vec![ClaudeMessage {
            role:    "user".to_string(),
            content: user_msg.to_string(),
        }],
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
struct ClaudeContent {
    text: String,
}

// ─────────────────────────────── Public types ────────────────────────────────

/// A single per-position recommendation from Claude.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRecommendation {
    pub symbol: String,
    /// "scale_up" | "hold" | "scale_down" | "close_now"
    pub action: String,
    /// Multiplier for the existing size.
    ///   scale_up:   1.2 – 2.0  (add 20%–100% to position)
    ///   hold:       1.0
    ///   scale_down: 0.25 – 0.75 (keep 25%–75% of position)
    ///   close_now:  0.0
    pub factor: f64,
    pub reason: String,
}

/// Full review returned by `review_positions()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiReview {
    pub analysis:        String,
    pub recommendations: Vec<AiRecommendation>,
}

// ─────────────────────────────── Context snapshot ────────────────────────────

#[derive(Debug, Serialize)]
struct PositionContext {
    symbol:              String,
    side:                String,
    entry_price:         f64,
    current_price:       f64,
    r_multiple:          f64,
    unrealised_pnl_usd:  f64,
    margin_usd:          f64,
    leverage:            f64,
    notional_usd:        f64,
    hold_time_minutes:   u64,
    dca_count:           u8,
    /// Remaining DCA add-ons available (max 2 minus used).
    /// IMPORTANT: if dca_remaining > 0 the strategy has a built-in rescue
    /// mechanism — do NOT recommend close_now just because R is negative.
    dca_remaining:       u8,
    stop_loss:           f64,
    take_profit:         f64,
    tranches_closed:     u8,
}

// ─────────────────────────────── Main function ───────────────────────────────

/// Call Claude to review open positions and return scaling recommendations.
///
/// Returns `Ok(AiReview)` with empty recommendations if there are no open
/// positions or if the API call fails non-fatally (errors are logged, not
/// propagated, so a transient API issue never crashes the trading loop).
pub async fn review_positions(
    positions: &[PaperPosition],
    metrics:   &PerformanceMetrics,
    capital:   f64,
    api_key:   &str,
) -> AiReview {
    match review_inner(positions, metrics, capital, api_key).await {
        Ok(review) => review,
        Err(e) => {
            warn!("🤖 AI review skipped: {}", e);
            AiReview {
                analysis:        format!("Review unavailable: {}", e),
                recommendations: vec![],
            }
        }
    }
}

async fn review_inner(
    positions: &[PaperPosition],
    metrics:   &PerformanceMetrics,
    capital:   f64,
    api_key:   &str,
) -> Result<AiReview> {
    if positions.is_empty() {
        return Ok(AiReview {
            analysis:        "No open positions.".to_string(),
            recommendations: vec![],
        });
    }

    // ── Build position context ───────────────────────────────────────────
    let ctx: Vec<PositionContext> = positions.iter().map(|p| {
        let r_mult = if p.r_dollars_risked > 1e-8 {
            p.unrealised_pnl / p.r_dollars_risked
        } else {
            0.0
        };
        // Approximate current price from PnL (works for both LONG and SHORT)
        let cur_price = if p.quantity > 1e-10 {
            let price_delta = p.unrealised_pnl / p.quantity;
            if p.side == "LONG" { p.entry_price + price_delta }
            else                 { p.entry_price - price_delta }
        } else {
            p.entry_price
        };
        PositionContext {
            symbol:             p.symbol.clone(),
            side:               p.side.clone(),
            entry_price:        p.entry_price,
            current_price:      cur_price,
            r_multiple:         (r_mult * 100.0).round() / 100.0,
            unrealised_pnl_usd: (p.unrealised_pnl * 100.0).round() / 100.0,
            margin_usd:         (p.size_usd * 100.0).round() / 100.0,
            leverage:           p.leverage,
            notional_usd:       (p.size_usd * p.leverage * 100.0).round() / 100.0,
            hold_time_minutes:  p.cycles_held / 2,
            dca_count:          p.dca_count,
            dca_remaining:      2u8.saturating_sub(p.dca_count),
            stop_loss:          p.stop_loss,
            take_profit:        p.take_profit,
            tranches_closed:    p.tranches_closed,
        }
    }).collect();

    let pos_json = serde_json::to_string_pretty(&ctx)
        .map_err(|e| anyhow!("Serialisation error: {}", e))?;

    let portfolio_summary = format!(
        "Free capital: ${:.2} | Win rate: {:.1}% | Expectancy: {:.2}% | Sharpe: {:.2} | Max DD: {:.1}% | Profit factor: {:.2}",
        capital,
        metrics.win_rate * 100.0,
        metrics.expectancy,
        metrics.sharpe,
        metrics.max_drawdown * 100.0,
        metrics.profit_factor,
    );

    let user_msg = format!(
        "Portfolio snapshot:\n{}\n\nOpen positions (JSON):\n{}\n\n\
         Review each position. Be concise — one clear recommendation per position.",
        portfolio_summary,
        pos_json,
    );

    // ── System prompt ────────────────────────────────────────────────────
    let system_prompt = r#"You are an aggressive crypto perpetuals trading bot manager. Your goal is to maximise returns — that means actively scaling winners, cutting losers early, and never letting profits bleed away. You are NOT a conservative risk manager. You are here to make money.

## Your four actions

**scale_up (factor 1.3–3.0) — DO THIS OFTEN**
When a position is working, add to it. Requirements: R > 0.3, hold_time > 20 min, price trending your direction.
Use factors: 1.3–1.5 at R=0.3–0.6, 1.5–2.0 at R=0.6–1.5, up to 3.0 at R > 1.5 with strong momentum.
Don't wait for "perfect" — scale up early on winners.

**hold (factor 1.0)**
Use when the position is in early stages (< 20 min), R is mildly negative but still inside normal noise (-0.2R to 0), or momentum is unclear. Not your default — only use it when you genuinely can't decide.

**scale_down (factor 0.3–0.7)**
When position is stagnant (|R| < 0.1 for 60+ min) or showing early signs of failure. Reduce to free up capital for better trades. dca_remaining being 0 is NOT required — if the trade isn't working, shrink it.

**close_now (factor 0.0)**
Cut losers fast. Use when:
  - hold_time > 20 min AND r_multiple < -0.30 AND price action is clearly going against you
  - OR hold_time > 30 min AND r_multiple < -0.50 (deep loss regardless of DCA)
  - OR dca_count >= 1 AND r_multiple < -0.35 (DCA already tried, still losing)
Don't let a bad trade bleed — close it and redeploy the capital.

## Strategy context
- Partial exits: 1/3 auto-closes at 2R, 1/3 at 4R, trailing stop on the rest. You don't need to manage normal winners — the system handles them.
- DCA: up to 2 adds available, but DCA is not a reason to hold a losing trade indefinitely.
- Leverage: 2× to 5×. A -0.3R position at 5× leverage is a real loss — treat it seriously.
- Your edge: scale winners aggressively and cut losers before they become big losses.

## What to avoid
- Holding a losing trade just because dca_remaining > 0 — DCA doesn't fix broken signals
- Waiting too long to scale up winners — profits have a way of disappearing in crypto
- Recommending "hold" for everything — that's not managing, that's ignoring

Respond with ONLY valid JSON (no markdown, no preamble):
{
  "analysis": "One sentence: portfolio health + any dominant theme.",
  "recommendations": [
    {"symbol": "SOL", "action": "scale_up", "factor": 1.5, "reason": "R=0.7, 35 min held, momentum strong — adding to winner."},
    {"symbol": "BTC", "action": "close_now", "factor": 0.0, "reason": "R=-0.38, 25 min held, price breaking support — cut the loss."}
  ]
}"#;

    // ── API call ─────────────────────────────────────────────────────────
    let request = ClaudeRequest {
        model:      "claude-haiku-4-5-20251001".to_string(),
        max_tokens: 1000,
        system:     system_prompt.to_string(),
        messages:   vec![ClaudeMessage {
            role:    "user".to_string(),
            content: user_msg,
        }],
    };

    let client = Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key",          api_key)
        .header("anthropic-version",  "2023-06-01")
        .header("content-type",       "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow!("HTTP error: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body   = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Claude API {} — {}", status, &body[..body.len().min(200)]));
    }

    let claude_resp: ClaudeResponse = resp.json().await
        .map_err(|e| anyhow!("JSON decode error: {}", e))?;

    let text = claude_resp.content
        .into_iter()
        .find(|c| !c.text.is_empty())
        .map(|c| c.text)
        .ok_or_else(|| anyhow!("Empty response from Claude"))?;

    // Strip any markdown fences Claude might add despite instructions
    let json_str = text.trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let review: AiReview = serde_json::from_str(json_str)
        .map_err(|e| anyhow!("Parse error: {} — raw response: {}", e, &json_str[..json_str.len().min(300)]))?;

    // ── Log results ──────────────────────────────────────────────────────
    info!("🤖 AI Review: {}", review.analysis);
    for rec in &review.recommendations {
        let icon = match rec.action.as_str() {
            "scale_up"   => "📈",
            "scale_down" => "📉",
            "close_now"  => "🛑",
            _            => "⏸",
        };
        info!("  {} {} → {} ×{:.2}  — {}",
            icon, rec.symbol, rec.action, rec.factor, rec.reason);
    }

    Ok(review)
}
