//! `handlers_public` — part of the `web_dashboard` module tree.
//!
//! Shared types and helpers available via `use super::*;`.
#![allow(unused_imports)]

use super::*;

pub(crate) async fn apple_pay_domain_handler(State(app): State<AppState>) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    match &app.apple_pay_domain_assoc {
        Some(content) => (
            StatusCode::OK,
            [("Content-Type", "text/plain; charset=utf-8")],
            content.clone(),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "Apple Pay domain association file not configured.\n\
             Set APPLE_PAY_DOMAIN_ASSOC in your environment.",
        )
            .into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Public TVL API — no auth required, powers landing page hero graph
// ═══════════════════════════════════════════════════════════════════════════════

// `GET /api/public/tvl`
// ─────────────────────────────────────────────────────────────────────────────
// Leaderboard + campaign handlers
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /leaderboard` — public leaderboard page for the active campaign.
///
/// Shows the current standings, prize pool, countdown timer, and how to get
/// an invite code.  No authentication required — it's a viral acquisition page.
pub(crate) async fn leaderboard_handler(State(app): State<AppState>) -> axum::response::Html<String> {
    let (campaign, entries) = match &app.db {
        Some(db) => {
            let c = crate::leaderboard::active_campaign(db).await.ok().flatten();
            let e = if c.is_some() {
                crate::leaderboard::live_standings(db, 50)
                    .await
                    .unwrap_or_default()
            } else {
                vec![]
            };
            (c, e)
        }
        None => (None, vec![]),
    };

    let campaign_title = campaign
        .as_ref()
        .map(|c| c.title.clone())
        .unwrap_or_else(|| "Weekly Trading Contest".into());
    let campaign_desc = campaign
        .as_ref()
        .and_then(|c| c.description.clone())
        .unwrap_or_else(|| "Top traders by % return win weekly prizes.".into());
    let prize_pool = campaign.as_ref().map(|c| c.prize_pool_usd).unwrap_or(0.0);
    let seconds_left = campaign.as_ref().map(|c| c.seconds_left).unwrap_or(0);

    let prizes_html = campaign.as_ref().map(|c| {
        c.prizes.iter().map(|p| format!(
            r#"<div class="prize-row"><span class="prize-label">{}</span><span class="prize-amt">${}</span></div>"#,
            p.label, p.usd as i64
        )).collect::<Vec<_>>().join("")
    }).unwrap_or_default();

    let rows_html: String = if entries.is_empty() {
        r#"<tr><td colspan="5" style="text-align:center;color:#484f58;padding:32px">No trades recorded yet this week — be the first!</td></tr>"#.into()
    } else {
        entries
            .iter()
            .map(|e| {
                let medal = match e.rank {
                    1 => "🥇",
                    2 => "🥈",
                    3 => "🥉",
                    _ => "",
                };
                let pct_color = if e.pct_return >= 0.0 {
                    "#3fb950"
                } else {
                    "#f85149"
                };
                let pct_sign = if e.pct_return >= 0.0 { "+" } else { "" };
                format!(
                    r#"<tr class="lb-row{rank_cls}">
                  <td class="lb-rank">{medal}{rank}</td>
                  <td class="lb-name">{name}</td>
                  <td class="lb-wallet">{wallet}</td>
                  <td class="lb-trades">{trades}</td>
                  <td class="lb-pct" style="color:{pct_color}">{pct_sign}{pct:.2}%</td>
                </tr>"#,
                    rank_cls = if e.rank <= 3 { " top3" } else { "" },
                    medal = medal,
                    rank = e.rank,
                    name = html_escape(&e.display_name),
                    wallet = e.wallet_short,
                    trades = e.trades_in_period,
                    pct_color = pct_color,
                    pct_sign = pct_sign,
                    pct = e.pct_return,
                )
            })
            .collect()
    };

    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Leaderboard</title>
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#0d1117;color:#c9d1d9;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;min-height:100vh;padding:0 0 60px}}
.hero{{background:linear-gradient(155deg,#161b22,#0d1117);border-bottom:1px solid #21262d;padding:48px 24px 40px;text-align:center}}
.hero-badge{{display:inline-block;background:rgba(255,215,0,.12);border:1px solid rgba(255,215,0,.3);border-radius:20px;padding:5px 14px;font-size:.72rem;font-weight:700;color:#ffd700;letter-spacing:.8px;text-transform:uppercase;margin-bottom:16px}}
.hero h1{{font-size:2rem;font-weight:800;color:#e6edf3;margin-bottom:8px}}
.hero h1 .g{{color:#3fb950}}.hero h1 .r{{color:#e6343a}}
.hero-sub{{font-size:.9rem;color:#8b949e;max-width:500px;margin:0 auto 24px}}
.prize-bar{{display:flex;justify-content:center;gap:16px;flex-wrap:wrap;margin-bottom:28px}}
.prize-row{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:12px 20px;text-align:center;min-width:100px}}
.prize-label{{display:block;font-size:.75rem;color:#8b949e;margin-bottom:4px}}
.prize-amt{{display:block;font-size:1.3rem;font-weight:800;color:#ffd700}}
.countdown{{font-size:.82rem;color:#484f58;margin-top:8px}}
.countdown span{{color:#58a6ff;font-weight:700}}
.cta-strip{{background:rgba(63,185,80,.07);border:1px solid rgba(63,185,80,.2);border-radius:12px;padding:20px 24px;max-width:520px;margin:0 auto;text-align:left}}
.cta-strip h3{{font-size:.92rem;font-weight:700;color:#e6edf3;margin-bottom:6px}}
.cta-strip p{{font-size:.78rem;color:#8b949e;line-height:1.6;margin-bottom:12px}}
.cta-strip .how{{font-size:.75rem;color:#6e7681;line-height:1.8}}
.cta-strip .how b{{color:#c9d1d9}}
.btn-signin{{display:inline-block;padding:11px 24px;background:#3fb950;color:#0d1117;border-radius:8px;font-weight:700;font-size:.88rem;text-decoration:none;transition:.15s}}
.btn-signin:hover{{background:#52c965}}
.wrap{{max-width:860px;margin:0 auto;padding:32px 20px 0}}
.lb-wrap{{background:#161b22;border:1px solid #21262d;border-radius:14px;overflow:hidden}}
.lb-hd{{padding:18px 22px;border-bottom:1px solid #21262d;display:flex;align-items:center;justify-content:space-between}}
.lb-hd-title{{font-size:.92rem;font-weight:700;color:#e6edf3}}
.lb-hd-sub{{font-size:.72rem;color:#484f58}}
table{{width:100%;border-collapse:collapse}}
th{{padding:10px 16px;font-size:.7rem;font-weight:700;color:#484f58;text-transform:uppercase;letter-spacing:.6px;text-align:left;border-bottom:1px solid #21262d}}
.lb-row td{{padding:13px 16px;font-size:.85rem;border-bottom:1px solid rgba(48,54,61,.5);transition:.1s}}
.lb-row:hover td{{background:rgba(255,255,255,.02)}}
.lb-row.top3 td{{background:rgba(255,215,0,.03)}}
.lb-rank{{font-weight:700;color:#e6edf3;width:60px}}
.lb-name{{color:#c9d1d9;font-weight:600}}
.lb-wallet{{color:#484f58;font-size:.78rem;font-family:monospace}}
.lb-trades{{color:#8b949e;text-align:center;width:80px}}
.lb-pct{{font-weight:700;text-align:right;width:100px}}
.pool-badge{{display:inline-block;background:rgba(255,215,0,.12);border:1px solid rgba(255,215,0,.25);border-radius:8px;padding:4px 10px;font-size:.8rem;color:#ffd700;font-weight:700}}
</style>
</head>
<body>

<!-- Hero -->
<div class="hero">
  <div class="hero-badge">🏆 Weekly Contest</div>
  <h1>TradingBots<span class="g">.fun</span> Leaderboard</h1>
  <p class="hero-sub">{desc}</p>

  <div class="prize-bar">
    {prizes_html}
  </div>
  <div class="countdown" id="countdown">Prize pool: <span class="pool-badge">${prize_pool}</span></div>

  <!-- How to join -->
  <div class="cta-strip" style="margin-top:28px">
    <h3>🎟 How to join</h3>
    <p>TradingBots.fun is invite-only. Get a code from a friend, enter it on the sign-in page, deposit as little as <b style="color:#e6edf3">$20</b>, and let two bots trade for you. Best % return wins.</p>
    <div class="how">
      <b>1.</b> Get an invite code from a friend or this leaderboard ·
      <b>2.</b> Sign in at <a href="/login" style="color:#58a6ff">/login</a> ·
      <b>3.</b> Deposit $20+ to Hyperliquid ·
      <b>4.</b> Two bots start automatically · <b>5.</b> Compete
    </div>
    <br>
    <a href="/login" class="btn-signin">Get started →</a>
  </div>
</div>

<!-- Standings -->
<div class="wrap">
  <div class="lb-wrap">
    <div class="lb-hd">
      <span class="lb-hd-title">{title} · Current Standings</span>
      <span class="lb-hd-sub">Ranked by % return · any deposit size competes equally</span>
    </div>
    <table>
      <thead>
        <tr>
          <th>Rank</th>
          <th>Trader</th>
          <th>Wallet</th>
          <th style="text-align:center">Trades</th>
          <th style="text-align:right">Return</th>
        </tr>
      </thead>
      <tbody>{rows_html}</tbody>
    </table>
  </div>
</div>

<script>
// Countdown timer
const secsLeft = {seconds_left};
function fmt(s) {{
  if (s <= 0) return 'Contest ended';
  const d = Math.floor(s/86400), h = Math.floor((s%86400)/3600),
        m = Math.floor((s%3600)/60), ss = s%60;
  if (d > 0) return d+'d '+h+'h '+m+'m left';
  if (h > 0) return h+'h '+m+'m '+ss+'s left';
  return m+'m '+ss+'s left';
}}
let remaining = secsLeft;
const el = document.getElementById('countdown');
function tick() {{
  const pool = el.querySelector('.pool-badge');
  const poolHtml = pool ? pool.outerHTML : '';
  el.innerHTML = 'Prize pool: '+poolHtml+'  ·  <span style="color:#58a6ff;font-weight:700">'+fmt(remaining)+'</span>';
  remaining--;
  if (remaining >= 0) setTimeout(tick, 1000);
}}
if (secsLeft > 0) tick();
</script>
</body></html>"#,
        title = html_escape(&campaign_title),
        desc = html_escape(&campaign_desc),
        prizes_html = prizes_html,
        prize_pool = prize_pool as i64,
        rows_html = rows_html,
        seconds_left = seconds_left,
    ))
}

/// `POST /app/invite/generate` — authenticated endpoint.
///
/// Generates a personal referral code for the logged-in tenant and returns it.
/// The code is valid for 30 days, single-use, and tied to the active campaign.
pub(crate) async fn generate_invite_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let tenant_id = match crate::privy::require_tenant_id(&headers, &app.session_secret) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error":"Unauthorized"})),
            )
                .into_response()
        }
    };

    let db = match &app.db {
        Some(db) => db,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({"error":"Database not configured"})),
            )
                .into_response()
        }
    };

    match crate::invite::generate_referral_code(db, &tenant_id).await {
        Ok(code) => axum::Json(serde_json::json!({
            "ok": true,
            "code": code,
            "share_url": format!("/login?invite={}", code),
            "expires_days": 30,
        }))
        .into_response(),
        Err(e) => {
            log::error!("generate_invite_handler: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error":"Could not generate code"})),
            )
                .into_response()
        }
    }
}

/// `GET /app/invite` — returns the tenant's current referral code (or generates one).
pub(crate) async fn get_invite_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let tenant_id = match crate::privy::require_tenant_id(&headers, &app.session_secret) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error":"Unauthorized"})),
            )
                .into_response()
        }
    };

    let db = match &app.db {
        Some(db) => db,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({"error":"Database not configured"})),
            )
                .into_response()
        }
    };

    let code = match crate::invite::get_referral_code_for_tenant(db, &tenant_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            // Auto-generate on first request
            match crate::invite::generate_referral_code(db, &tenant_id).await {
                Ok(c) => c,
                Err(e) => {
                    log::error!("get_invite auto-generate: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        axum::Json(serde_json::json!({"error":"Could not generate code"})),
                    )
                        .into_response();
                }
            }
        }
        Err(e) => {
            log::error!("get_invite_handler: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error":"DB error"})),
            )
                .into_response();
        }
    };

    axum::Json(serde_json::json!({
        "ok": true,
        "code": code,
        "share_url": format!("/login?invite={}", code),
    }))
    .into_response()
}

// ── Live winning-trade SSE stream ────────────────────────────────────────────

/// `GET /api/trade-stream` — Server-Sent Events stream of winning trade closes.
///
/// Browsers connect with `new EventSource('/api/trade-stream')` and receive
/// one JSON event per winning (or partial-profit) trade close.  A keep-alive
/// comment is sent every 15 s to prevent proxy timeouts.
///
/// Event format (`data` field is JSON):
/// ```json
/// {"symbol":"ETH","side":"LONG","pnl":12.40,"pnl_pct":3.1,"r_mult":1.25,
///  "reason":"Partial1.25R","wallet":"Bot Alpha","closed_at":"2026-03-26T…"}
/// ```
pub(crate) async fn trade_stream_handler() -> Sse<impl futures_util::stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::wrappers::BroadcastStream;

    let rx = crate::trade_stream::subscribe()
        .expect("trade_stream not initialised — call trade_stream::init() at startup");

    let stream = BroadcastStream::new(rx).filter_map(|msg| async move {
        match msg {
            Ok(win) => {
                let data = serde_json::to_string(&win).unwrap_or_default();
                Some(Ok(Event::default().event("trade_win").data(data)))
            }
            // Receiver lagged (burst): skip the missed events and continue.
            Err(_) => None,
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
}

/// `GET /api/leaderboard` — JSON endpoint for the current standings.
pub(crate) async fn api_leaderboard_handler(State(app): State<AppState>) -> impl axum::response::IntoResponse {
    use axum::response::IntoResponse;
    let db = match &app.db {
        Some(db) => db,
        None => {
            return axum::Json(serde_json::json!({"entries":[],"campaign":null})).into_response()
        }
    };
    let campaign = crate::leaderboard::active_campaign(db).await.ok().flatten();
    let entries = crate::leaderboard::live_standings(db, 100)
        .await
        .unwrap_or_default();
    axum::Json(serde_json::json!({ "campaign": campaign, "entries": entries })).into_response()
}

///
/// Returns the last 90 days of AUM snapshots as JSON.
/// Used by the landing page to render the TVL hero graph client-side.
/// No authentication required — returns aggregate data only, never per-tenant.
pub(crate) async fn public_tvl_handler(State(app): State<AppState>) -> impl axum::response::IntoResponse {
    use axum::http::{HeaderMap, StatusCode};

    let mut headers = HeaderMap::new();
    // Allow embedding in the landing page (different origin during dev).
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
    headers.insert("Cache-Control", "public, max-age=60".parse().unwrap());

    let Some(db) = &app.db else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            headers,
            axum::Json(serde_json::json!({
                "error": "database not yet configured",
                "points": [],
            })),
        );
    };

    let points = match db.get_aum_history(90).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("public_tvl_handler: DB error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                headers,
                axum::Json(serde_json::json!({ "error": "query failed", "points": [] })),
            );
        }
    };

    // Pull the latest snapshot for the headline numbers.
    let latest = db.get_latest_aum().await.ok().flatten();

    let response = serde_json::json!({
        "generated_at":    chrono::Utc::now().to_rfc3339(),
        "window_days":     90,
        "point_count":     points.len(),
        "latest": latest.as_ref().map(|a| serde_json::json!({
            "total_aum":         a.total_aum,
            "deposited_capital": a.deposited_capital,
            "total_pnl":         a.total_pnl,
            "pnl_pct":           a.pnl_pct,
            "active_tenants":    a.active_tenants,
            "total_tenants":     a.total_tenants,
            "open_positions":    a.open_positions,
            "recorded_at":       a.recorded_at.to_rfc3339(),
        })),
        "points": points.iter().map(|p| serde_json::json!({
            "ts":          p.recorded_at.to_rfc3339(),
            "aum":         p.total_aum,
            "pnl":         p.total_pnl,
            "pnl_pct":     p.pnl_pct,
            "tenants":     p.active_tenants,
            "positions":   p.open_positions,
        })).collect::<Vec<_>>(),
    });

    (StatusCode::OK, headers, axum::Json(response))
}

/// `GET /api/public/tvl/svg`
///
/// Returns a self-contained SVG sparkline of the TVL curve.
/// Embed directly in the landing page `<img src="/api/public/tvl/svg">` —
/// no JavaScript required.  Auto-updates every 60 seconds via HTTP cache.
pub(crate) async fn public_tvl_svg_handler(State(app): State<AppState>) -> impl axum::response::IntoResponse {
    use axum::http::{HeaderMap, StatusCode};

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "image/svg+xml".parse().unwrap());
    headers.insert("Cache-Control", "public, max-age=60".parse().unwrap());
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());

    let placeholder_svg = r##"<svg width="480" height="80" viewBox="0 0 480 80"
         xmlns="http://www.w3.org/2000/svg"
         style="background:#0d1117;border-radius:8px">
  <text x="240" y="45" text-anchor="middle" fill="#484f58"
        font-family="system-ui,sans-serif" font-size="13">
    Accumulating data…
  </text>
</svg>"##;

    let Some(db) = &app.db else {
        return (StatusCode::OK, headers, placeholder_svg.to_string());
    };

    let points = match db.get_aum_history(90).await {
        Ok(p) if p.len() >= 2 => p,
        _ => return (StatusCode::OK, headers, placeholder_svg.to_string()),
    };

    // Build SVG using the same proven pattern as the equity sparkline.
    let w_px: f64 = 480.0;
    let h_px: f64 = 80.0;
    let pad: f64 = 8.0;
    let inner_h = h_px - 2.0 * pad;

    let values: Vec<f64> = points.iter().map(|p| p.total_aum).collect();
    let deposited = points.first().map(|p| p.deposited_capital).unwrap_or(0.0);

    let data_min = values
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min)
        .min(deposited);
    let data_max = values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(deposited);
    let buf = ((data_max - data_min).max(deposited * 0.002)) * 0.15;
    let min_v = data_min - buf;
    let max_v = data_max + buf;
    let range = (max_v - min_v).max(0.01);

    let to_y = |v: f64| h_px - pad - (v - min_v) / range * inner_h;

    let n = values.len() as f64;
    let pts: String = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f64 / (n - 1.0) * w_px;
            let y = to_y(v);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    let base_y = to_y(deposited);
    let last_y = to_y(*values.last().unwrap());
    let last_v = *values.last().unwrap();
    let trend_c = if last_v >= deposited {
        "#3fb950"
    } else {
        "#f85149"
    };
    let fill_pts = format!("{pts} {w_px:.1},{base_y:.1} 0.0,{base_y:.1}");

    let latest_pnl_pct = points.last().map(|p| p.pnl_pct).unwrap_or(0.0);
    let pnl_sign = if latest_pnl_pct >= 0.0 { "+" } else { "" };
    let label = format!("{pnl_sign}{latest_pnl_pct:.1}% all-time");

    let svg = format!(
        r##"<svg width="480" height="80" viewBox="0 0 480 80"
     xmlns="http://www.w3.org/2000/svg"
     style="background:#0d1117;border-radius:8px;display:block">
  <line x1="0" y1="{by:.1}" x2="480" y2="{by:.1}"
        stroke="{c}" stroke-width="0.8" stroke-dasharray="3 3" stroke-opacity="0.4"/>
  <polygon points="{fp}" fill="{c}" fill-opacity="0.12"/>
  <polyline points="{pts}" fill="none" stroke="{c}"
            stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>
  <circle cx="480" cy="{ly:.1}" r="4" fill="{c}"/>
  <text x="8" y="20" font-family="system-ui,sans-serif" font-size="11"
        fill="{c}" font-weight="600">{label}</text>
</svg>"##,
        c = trend_c,
        by = base_y,
        fp = fill_pts,
        pts = pts,
        ly = last_y,
        label = label,
    );

    (StatusCode::OK, headers, svg)
}

// ─────────────────────────────────────────────────────────────────────────────
//  POST /api/funnel  — client-side event ingestion
// ─────────────────────────────────────────────────────────────────────────────

/// Accepts `navigator.sendBeacon` payloads from the client tracking script.
///
/// Validates the `event_type` against the known set, attaches the server-side
/// tenant context if the session cookie is present, then writes to `funnel_events`.
pub(crate) async fn funnel_event_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: axum::extract::Json<crate::funnel::FunnelEventPayload>,
) -> axum::http::StatusCode {
    use crate::funnel::{record, FunnelEvent};
    use axum::http::StatusCode;

    let payload = body.0;

    // Map the string event_type → enum (rejects unknown values)
    let event = match payload.event_type.as_str() {
        "PAGE_VIEW" => FunnelEvent::PageView,
        "LOGIN_CLICK" => FunnelEvent::LoginClick,
        "AUTH_SUCCESS" => FunnelEvent::AuthSuccess,
        "TRIAL_START" => FunnelEvent::TrialStart,
        "TERMS_ACCEPTED" => FunnelEvent::TermsAccepted,
        "WALLET_LINKED" => FunnelEvent::WalletLinked,
        "FIRST_POSITION" => FunnelEvent::FirstPosition,
        "UPGRADE_CLICK" => FunnelEvent::UpgradeClick,
        "CHECKOUT_STARTED" => FunnelEvent::CheckoutStarted,
        "UPGRADED" => FunnelEvent::Upgraded,
        "TRIAL_EXPIRED" => FunnelEvent::TrialExpired,
        "CHURNED" => FunnelEvent::Churned,
        "AD_IMPRESSION" => FunnelEvent::AdImpression,
        "AD_CLICK" => FunnelEvent::AdClick,
        _ => return StatusCode::BAD_REQUEST,
    };

    // Resolve tenant from session cookie if present (pre-auth events have None)
    // get_session_tenant_id already returns Option<TenantId> — no mapping needed
    let tid = get_session_tenant_id(&headers, &app.session_secret);

    record(
        &app.db,
        event,
        &payload.anon_id,
        tid.as_ref(),
        Some(payload.extra),
    )
    .await;

    StatusCode::NO_CONTENT
}

// ─────────────────────────── Trade journal ───────────────────────────────────

/// Payload for `POST /api/trade-note`.
#[derive(Debug, Deserialize)]
struct TradeNotePayload {
    /// Index into `bot_state.closed_trades` (0 = most recent).
    index: usize,
    /// Operator's plain-text note — max 500 chars.
    note: String,
}

/// `POST /api/trade-note` — attach an operator note to a closed trade.
///
/// The note is written to the in-memory `ClosedTrade` and also persisted to
/// the PostgreSQL `closed_trade_notes` table so it survives restarts.
///
/// Requires a valid admin session (checked via `ADMIN_PASSWORD`).
/// Returns 204 No Content on success, 400 on bad input, 404 if index OOB.
pub(crate) async fn trade_note_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: axum::extract::Json<TradeNotePayload>,
) -> axum::http::StatusCode {
    use axum::http::StatusCode;

    // Simple admin gate: require the same HTTP-Basic admin password used on /admin.
    // In production this endpoint is only hit from the admin panel JS.
    if let Some(pw) = &app.admin_password {
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Accept both "Basic <b64(admin:pw)>" and bare bearer token equal to password.
        let b64 = base64_encode(&format!("admin:{}", pw));
        let expected_basic = format!("Basic {}", b64);
        if auth != expected_basic && auth != pw.as_str() {
            return StatusCode::UNAUTHORIZED;
        }
    }

    let payload = body.0;

    // Validate note length.
    if payload.note.len() > 500 {
        return StatusCode::BAD_REQUEST;
    }

    // Write into in-memory state.
    {
        let mut state = app.bot_state.write().await;
        match state.closed_trades.get_mut(payload.index) {
            Some(trade) => {
                trade.note = Some(payload.note.clone());
            }
            None => return StatusCode::NOT_FOUND,
        }
    }

    // Persist to DB (best-effort — don't fail the request if DB is down).
    // Uses sqlx::query() (not macro) so migration 007 need not exist at compile time.
    if let Some(db) = &app.db {
        let idx = payload.index as i64;
        let note = payload.note.clone();
        let _ = sqlx::query(
            "INSERT INTO closed_trade_notes (trade_index, note, updated_at) \
             VALUES ($1, $2, NOW()) \
             ON CONFLICT (trade_index) DO UPDATE \
               SET note = EXCLUDED.note, updated_at = NOW()",
        )
        .bind(idx)
        .bind(note)
        .execute(db.pool())
        .await;
    }

    StatusCode::NO_CONTENT
}

#[derive(Debug, Deserialize)]
struct ReportQueryPayload {
    question: String,
    answer: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReportQueryResponse {
    answer: Option<String>,
    cached: bool,
    report_hash: String,
}

pub(crate) async fn api_report_latest_handler(State(app): State<AppState>, headers: HeaderMap) -> Result<Json<reporting::ReportSummary>, axum::http::StatusCode> {
    let has_session = get_session_tenant_id(&headers, &app.session_secret).is_some();
    let has_admin = app.admin_password.as_deref().map(|pw| check_admin_auth(&headers, pw)).unwrap_or(false);
    if !has_session && !has_admin { return Err(axum::http::StatusCode::UNAUTHORIZED); }
    match reporting::load_summary() {
        Ok(summary) => Ok(Json(summary)),
        Err(_) => Err(axum::http::StatusCode::NOT_FOUND),
    }
}

pub(crate) async fn api_report_query_handler(State(app): State<AppState>, headers: HeaderMap, Json(payload): Json<ReportQueryPayload>) -> Result<Json<ReportQueryResponse>, axum::http::StatusCode> {
    let has_session = get_session_tenant_id(&headers, &app.session_secret).is_some();
    let has_admin = app.admin_password.as_deref().map(|pw| check_admin_auth(&headers, pw)).unwrap_or(false);
    if !has_session && !has_admin { return Err(axum::http::StatusCode::UNAUTHORIZED); }
    if payload.question.trim().is_empty() {
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }
    let summary = reporting::load_summary().map_err(|_| axum::http::StatusCode::NOT_FOUND)?;
    let report_hash = summary.report_hash.clone();
    let mut cache = app.report_cache.lock().await;
    if let Some(answer) = payload.answer {
        cache.store(&payload.question, &report_hash, answer.clone());
        let _ = cache.save();
        return Ok(Json(ReportQueryResponse {
            answer: Some(answer),
            cached: false,
            report_hash,
        }));
    }
    if let Some(entry) = cache.lookup(&payload.question, &report_hash) {
        return Ok(Json(ReportQueryResponse {
            answer: Some(entry.answer.clone()),
            cached: true,
            report_hash,
        }));
    }
    Ok(Json(ReportQueryResponse {
        answer: None,
        cached: false,
        report_hash,
    }))
}

pub(crate) async fn api_report_patterns_handler(
    State(app): State<AppState>,
) -> Result<Json<pattern_insights::PatternInsights>, axum::http::StatusCode> {
    let cache = app.pattern_cache.lock().await;
    if let Some(insights) = cache.latest() {
        Ok(Json(insights))
    } else {
        Err(axum::http::StatusCode::NOT_FOUND)
    }
}

pub(crate) async fn api_pattern_alert_handler(
    State(_app): State<AppState>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let path = PathBuf::from("reports").join("pattern_cache_alert.json");
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(json) => Ok(Json(json)),
            Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        },
        Err(_) => Err(axum::http::StatusCode::NOT_FOUND),
    }
}

#[derive(Debug, Deserialize)]
struct BridgeWithdrawRequest {
    amount_usd: f64,
    destination: String,
}

pub(crate) async fn bridge_withdraw_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<BridgeWithdrawRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tenant_id = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    };

    match app
        .bridge_manager
        .request_withdrawal(&tenant_id, req.amount_usd, req.destination.trim())
        .await
    {
        Ok(record) => axum::response::Json(record.view()).into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            axum::response::Json(serde_json::json!({
                "error": "bridge_failed",
                "message": e.to_string(),
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn bridge_status_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tenant_id = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    };

    match app.bridge_manager.fetch_record(&id).await {
        Some(record) if record.tenant_id == tenant_id => {
            axum::response::Json(record.view()).into_response()
        }
        Some(_) => axum::http::StatusCode::FORBIDDEN.into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

/// Minimal base64 encoder (no external dep) — only used for the Basic-Auth check above.
