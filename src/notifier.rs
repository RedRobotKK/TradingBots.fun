//! Webhook notification module — fires on key bot events.
//!
//! Supports Discord, Slack, and any generic JSON webhook.  Telegram is
//! supported via `TELEGRAM_BOT_TOKEN` + `TELEGRAM_CHAT_ID` env vars.
//!
//! ## Configuration (set in /etc/environment on the VPS)
//!
//! | Env var             | Purpose                                                     |
//! |---------------------|-------------------------------------------------------------|
//! | `WEBHOOK_URL`       | Discord / Slack / generic webhook URL                      |
//! | `TELEGRAM_BOT_TOKEN`| Telegram bot token from @BotFather                         |
//! | `TELEGRAM_CHAT_ID`  | Telegram chat/channel ID (e.g. `-1001234567890`)           |
//!
//! At least one of `WEBHOOK_URL` or both Telegram vars must be set, otherwise
//! all send calls are silently no-ops (the bot keeps trading normally).
//!
//! ## Events fired
//!
//! | Event              | Trigger                                          |
//! |--------------------|--------------------------------------------------|
//! | Position Opened    | New position entered via `execute_paper_trade`   |
//! | Position Closed    | Any close: SL, TP, partial, AI, signal reversal  |
//! | Circuit Breaker    | CB activates (drawdown > 8 %)                    |
//! | AI Action          | AI reviewer recommends close/scale               |
//!
//! ## Discord embed format
//!
//! ```json
//! {
//!   "embeds": [{
//!     "title": "📈 LONG BTC-USD opened",
//!     "color": 3066993,
//!     "fields": [...]
//!   }]
//! }
//! ```

use anyhow::Result;
use log::warn;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

// Discord embed colours
const COLOR_GREEN:  u32 = 3_066_993;  // #2ECC71
const COLOR_RED:    u32 = 15_158_332; // #E74C3C
const COLOR_ORANGE: u32 = 15_105_570; // #E67E22
const COLOR_GREY:   u32 = 9_807_270;  // #95A5A6

// ─────────────────────────── Notifier ────────────────────────────────────────

#[derive(Clone)]
pub struct Notifier {
    client:       Client,
    webhook_url:  Option<String>,
    tg_token:     Option<String>,
    tg_chat_id:   Option<String>,
}

impl Notifier {
    /// Build from environment variables.  Returns `None` if no destination
    /// is configured — callers treat `None` as "notifications disabled".
    pub fn from_env() -> Option<Self> {
        let webhook_url = std::env::var("WEBHOOK_URL").ok();
        let tg_token    = std::env::var("TELEGRAM_BOT_TOKEN").ok();
        let tg_chat_id  = std::env::var("TELEGRAM_CHAT_ID").ok();

        if webhook_url.is_none() && (tg_token.is_none() || tg_chat_id.is_none()) {
            return None;
        }

        Some(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap_or_default(),
            webhook_url,
            tg_token,
            tg_chat_id,
        })
    }

    // ── Public event methods ──────────────────────────────────────────────

    /// A new position was opened.
    #[allow(clippy::too_many_arguments)]
    pub async fn position_opened(
        &self,
        symbol:     &str,
        side:       &str,
        entry:      f64,
        size_usd:   f64,
        confidence: f64,
        stop_loss:  f64,
        leverage:   f64,
    ) {
        let emoji  = if side == "LONG" { "📈" } else { "📉" };
        let color  = if side == "LONG" { COLOR_GREEN } else { COLOR_RED };
        let title  = format!("{} {} {} opened", emoji, side, symbol);
        let fields = vec![
            field("Entry",      &format!("${:.4}", entry)),
            field("Size",       &format!("${:.2}", size_usd)),
            field("Leverage",   &format!("{:.1}×", leverage)),
            field("Stop Loss",  &format!("${:.4}", stop_loss)),
            field("Confidence", &format!("{:.0}%", confidence * 100.0)),
        ];
        self.send(embed(&title, color, fields)).await;
    }

    /// A position was fully or partially closed.
    pub async fn position_closed(
        &self,
        symbol:   &str,
        side:     &str,
        pnl:      f64,
        pnl_pct:  f64,
        reason:   &str,
        r_mult:   f64,
    ) {
        let emoji = if pnl >= 0.0 { "✅" } else { "❌" };
        let color = if pnl >= 0.0 { COLOR_GREEN } else { COLOR_RED };
        let title = format!("{} {} {} closed — {}", emoji, side, symbol, reason);
        let pnl_str = format!("{}{:.2} ({:+.1}%)",
            if pnl >= 0.0 { "+" } else { "" }, pnl, pnl_pct);
        let fields = vec![
            field("P&L",    &pnl_str),
            field("R-mult", &format!("{:.2}R", r_mult)),
            field("Reason", reason),
        ];
        self.send(embed(&title, color, fields)).await;
    }

    /// Circuit breaker fired.
    pub async fn circuit_breaker(&self, drawdown_pct: f64, size_mult: f64) {
        let title = "🔴 Circuit Breaker Activated".to_string();
        let fields = vec![
            field("7d Drawdown", &format!("{:.1}%", drawdown_pct)),
            field("New Size Mult", &format!("{:.2}×", size_mult)),
            field("Action", "Position sizes scaled down until equity recovers"),
        ];
        self.send(embed(&title, COLOR_ORANGE, fields)).await;
    }

    /// AI reviewer took an action on a position.
    pub async fn ai_action(
        &self,
        symbol: &str,
        action: &str,
        reason: &str,
        r_mult: f64,
    ) {
        let emoji = match action {
            "close_now"   => "🤖❌",
            "scale_up"    => "🤖📈",
            "scale_down"  => "🤖📉",
            _             => "🤖",
        };
        let title = format!("{} AI {} on {}", emoji, action.replace('_', " "), symbol);
        let fields = vec![
            field("R-multiple", &format!("{:.2}R", r_mult)),
            field("Reason", reason),
        ];
        self.send(embed(&title, COLOR_GREY, fields)).await;
    }

    // ── Internal dispatch ─────────────────────────────────────────────────

    async fn send(&self, payload: Value) {
        // Discord / Slack / generic webhook
        if let Some(url) = &self.webhook_url {
            if let Err(e) = self.post_discord(url, &payload).await {
                warn!("🔔 Webhook send failed: {e}");
            }
        }
        // Telegram
        if let (Some(token), Some(chat_id)) = (&self.tg_token, &self.tg_chat_id) {
            let text = discord_embed_to_text(&payload);
            if let Err(e) = self.post_telegram(token, chat_id, &text).await {
                warn!("🔔 Telegram send failed: {e}");
            }
        }
    }

    async fn post_discord(&self, url: &str, payload: &Value) -> Result<()> {
        self.client.post(url).json(payload).send().await?;
        Ok(())
    }

    async fn post_telegram(&self, token: &str, chat_id: &str, text: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
        self.client.post(&url)
            .json(&json!({ "chat_id": chat_id, "text": text, "parse_mode": "HTML" }))
            .send()
            .await?;
        Ok(())
    }
}

/// Shared reference — cloned into each async task that needs to fire events.
pub type SharedNotifier = std::sync::Arc<Notifier>;

// ─────────────────────────── Helpers ─────────────────────────────────────────

fn field(name: &str, value: &str) -> Value {
    json!({ "name": name, "value": value, "inline": true })
}

fn embed(title: &str, color: u32, fields: Vec<Value>) -> Value {
    json!({
        "embeds": [{
            "title":  title,
            "color":  color,
            "fields": fields,
            "footer": { "text": "TradingBots.fun" }
        }]
    })
}

/// Convert a Discord embed payload to a plain-text Telegram message.
fn discord_embed_to_text(payload: &Value) -> String {
    let title = payload["embeds"][0]["title"]
        .as_str()
        .unwrap_or("Bot event");
    let fields = payload["embeds"][0]["fields"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut lines = vec![format!("<b>{}</b>", title)];
    for f in fields {
        let name  = f["name"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");
        lines.push(format!("• <b>{}</b>: {}", name, value));
    }
    lines.join("\n")
}
