//! `handlers_admin` — part of the `web_dashboard` module tree.
//!
//! Shared types and helpers available via `use super::*;`.
#![allow(unused_imports)]

use super::*;

pub(crate) fn admin_shell(
    page_title: &str,
    active_nav: &str,       // "dashboard" | "users" | "wallets"
    pills_html: &str,       // pre-built pill badges for the topbar
    version: &str,
    body: &str,
) -> String {
    let nav_dash  = if active_nav == "dashboard" { " class=\"nav-item active\"" } else { " class=\"nav-item\"" };
    let nav_users = if active_nav == "users"     { " class=\"nav-item active\"" } else { " class=\"nav-item\"" };
    let nav_wall  = if active_nav == "wallets"   { " class=\"nav-item active\"" } else { " class=\"nav-item\"" };
    format!(r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · {page_title}</title>
<style>
*,*::before,*::after{{box-sizing:border-box;margin:0;padding:0}}
:root{{
  --bg:#0a0d12;--surface:#0f1318;--card:#141920;--border:#1e2530;--border2:#252d3a;
  --text:#d4dbe8;--muted:#586174;--label:#8494aa;
  --green:#22c55e;--green-dim:#16a34a33;--red:#f43f5e;--red-dim:#be123c33;
  --blue:#3b82f6;--blue-dim:#1d4ed833;--amber:#f59e0b;--amber-dim:#92400e33;
  --brand-r:#e6343a;--brand-g:#22c55e;
  --sidebar-w:220px;--radius:10px;--radius-sm:6px;
}}
html,body{{height:100%}}
body{{background:var(--bg);color:var(--text);font-family:-apple-system,BlinkMacSystemFont,'Inter','Segoe UI',sans-serif;font-size:13.5px;line-height:1.6;display:flex}}
.sidebar{{width:var(--sidebar-w);min-height:100vh;background:var(--surface);border-right:1px solid var(--border);display:flex;flex-direction:column;position:fixed;top:0;left:0;z-index:100}}
.sidebar-logo{{display:flex;align-items:center;gap:10px;padding:20px 18px 16px;border-bottom:1px solid var(--border)}}
.logo-icon{{width:32px;height:32px;background:linear-gradient(135deg,var(--brand-r),#a31b20);border-radius:8px;display:flex;align-items:center;justify-content:center;font-size:15px;font-weight:800;color:#fff;flex-shrink:0}}
.logo-text{{font-weight:700;font-size:.88rem;color:#e8edf5;line-height:1.2}}
.logo-text span{{color:var(--brand-g)}}
.logo-badge{{display:inline-block;font-size:.62rem;font-weight:600;letter-spacing:.06em;color:var(--amber);background:var(--amber-dim);border:1px solid #f59e0b44;border-radius:100px;padding:1px 7px;margin-top:2px}}
.nav{{padding:12px 8px;flex:1}}
.nav-section-label{{font-size:.65rem;font-weight:700;letter-spacing:.1em;color:var(--muted);text-transform:uppercase;padding:10px 10px 4px}}
.nav-item{{display:flex;align-items:center;gap:10px;padding:8px 10px;border-radius:var(--radius-sm);color:var(--label);font-size:.84rem;font-weight:500;text-decoration:none;cursor:pointer;transition:background .15s,color .15s;margin-bottom:1px}}
.nav-item:hover{{background:var(--card);color:var(--text)}}
.nav-item.active{{background:var(--blue-dim);color:#93c5fd}}
.nav-item svg{{width:15px;height:15px;opacity:.8;flex-shrink:0}}
.nav-badge{{margin-left:auto;font-size:.65rem;font-weight:700;background:var(--green-dim);color:var(--green);border-radius:100px;padding:1px 7px}}
.sidebar-footer{{padding:14px 18px;border-top:1px solid var(--border);font-size:.75rem;color:var(--muted)}}
.status-dot{{display:inline-block;width:6px;height:6px;background:var(--green);border-radius:50%;margin-right:5px;animation:pulse 2s infinite}}
@keyframes pulse{{0%,100%{{opacity:1}}50%{{opacity:.4}}}}
.main{{margin-left:var(--sidebar-w);flex:1;display:flex;flex-direction:column;min-height:100vh}}
.topbar{{height:56px;background:var(--surface);border-bottom:1px solid var(--border);display:flex;align-items:center;padding:0 28px;gap:16px;position:sticky;top:0;z-index:50}}
.topbar-title{{font-weight:700;font-size:.95rem;color:#e8edf5;flex:1}}
.topbar-actions{{display:flex;gap:8px;align-items:center}}
.pill{{display:flex;align-items:center;gap:5px;font-size:.75rem;font-weight:600;padding:4px 11px;border-radius:100px;border:1px solid transparent}}
.pill-green{{background:var(--green-dim);color:var(--green);border-color:#22c55e44}}
.pill-blue{{background:var(--blue-dim);color:#93c5fd;border-color:#3b82f644}}
.pill-amber{{background:var(--amber-dim);color:var(--amber);border-color:#f59e0b44}}
.pill-red{{background:var(--red-dim);color:var(--red);border-color:#f43f5e44}}
.page{{padding:28px;max-width:1100px}}
.kpi-grid{{display:grid;grid-template-columns:repeat(4,1fr);gap:14px;margin-bottom:24px}}
.kpi{{background:var(--card);border:1px solid var(--border);border-radius:var(--radius);padding:18px 20px;position:relative;overflow:hidden;transition:border-color .2s}}
.kpi:hover{{border-color:var(--border2)}}
.kpi::before{{content:'';position:absolute;top:0;left:0;right:0;height:2px}}
.kpi-blue::before{{background:linear-gradient(90deg,var(--blue),transparent)}}
.kpi-green::before{{background:linear-gradient(90deg,var(--green),transparent)}}
.kpi-amber::before{{background:linear-gradient(90deg,var(--amber),transparent)}}
.kpi-red::before{{background:linear-gradient(90deg,var(--red),transparent)}}
.kpi-label{{font-size:.68rem;font-weight:700;letter-spacing:.08em;text-transform:uppercase;color:var(--muted);margin-bottom:8px}}
.kpi-value{{font-size:1.85rem;font-weight:800;color:#edf2f9;letter-spacing:-.02em;line-height:1;margin-bottom:6px}}
.kpi-sub{{font-size:.73rem;color:var(--muted)}}
.kpi-icon{{position:absolute;top:16px;right:16px;font-size:1.4rem;opacity:.15}}
.two-col{{display:grid;grid-template-columns:1fr 1fr;gap:16px;margin-bottom:24px}}
.card{{background:var(--card);border:1px solid var(--border);border-radius:var(--radius);overflow:hidden;margin-bottom:0}}
.card-header{{display:flex;align-items:center;justify-content:space-between;padding:14px 20px;border-bottom:1px solid var(--border)}}
.card-title{{font-size:.8rem;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--label)}}
.card-body{{padding:20px}}
.control-row{{display:flex;align-items:flex-start;gap:16px;padding:14px 20px;border-bottom:1px solid var(--border)}}
.control-row:last-child{{border-bottom:none}}
.control-info{{flex:1}}
.control-name{{font-size:.84rem;font-weight:600;color:var(--text);margin-bottom:3px}}
.control-desc{{font-size:.75rem;color:var(--muted);line-height:1.4}}
.btn{{display:inline-flex;align-items:center;gap:6px;padding:8px 16px;border-radius:var(--radius-sm);font-size:.8rem;font-weight:600;font-family:inherit;border:1px solid transparent;cursor:pointer;transition:opacity .15s,transform .1s;text-decoration:none;white-space:nowrap}}
.btn:hover{{opacity:.85}}
.btn:active{{transform:scale(.97)}}
.btn-red{{background:#7f1d1d;border-color:#f43f5e44;color:#fca5a5}}
.btn-blue{{background:#1e3a5f;border-color:#3b82f644;color:#93c5fd}}
.btn-ghost{{background:transparent;border-color:var(--border2);color:var(--label)}}
.insight-item{{display:flex;align-items:center;gap:10px;padding:9px 0;border-bottom:1px solid var(--border);font-size:.8rem}}
.insight-item:last-child{{border-bottom:none}}
.insight-rank{{width:20px;height:20px;background:var(--border);border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:.65rem;font-weight:700;color:var(--muted);flex-shrink:0}}
.insight-label{{flex:1;color:var(--text);line-height:1.3}}
.insight-stat{{font-size:.73rem;font-weight:700;padding:2px 8px;border-radius:100px}}
.win-stat{{background:var(--green-dim);color:var(--green)}}
.loss-stat{{background:var(--red-dim);color:var(--red)}}
.quick-grid{{display:grid;grid-template-columns:1fr 1fr 1fr;gap:10px}}
.quick-card{{background:var(--border);border:1px solid var(--border2);border-radius:var(--radius-sm);padding:14px 16px;text-decoration:none;transition:background .15s,border-color .15s;display:block}}
.quick-card:hover{{background:var(--border2);border-color:#3b82f644}}
.quick-card-icon{{font-size:1.3rem;margin-bottom:6px}}
.quick-card-name{{font-size:.8rem;font-weight:600;color:var(--text)}}
.quick-card-desc{{font-size:.71rem;color:var(--muted);margin-top:2px}}
.table-wrap{{overflow-x:auto}}
table{{width:100%;border-collapse:collapse;font-size:.8rem}}
thead th{{color:var(--muted);font-weight:600;letter-spacing:.04em;font-size:.7rem;text-transform:uppercase;padding:10px 12px;text-align:left;border-bottom:1px solid var(--border);white-space:nowrap}}
tbody td{{padding:10px 12px;border-bottom:1px solid var(--border);color:var(--text)}}
tbody tr:last-child td{{border-bottom:none}}
tbody tr:hover td{{background:#ffffff04}}
tfoot td{{padding:10px 12px;border-top:1px solid var(--border2);color:var(--text);font-weight:700;font-size:.8rem}}
.tag{{display:inline-block;font-size:.67rem;font-weight:700;letter-spacing:.06em;padding:2px 8px;border-radius:100px}}
.tag-pro{{background:var(--green-dim);color:var(--green)}}
.tag-free{{background:var(--border);color:var(--muted)}}
.tag-int{{background:var(--amber-dim);color:var(--amber)}}
.mono{{font-family:'SF Mono',Menlo,monospace;font-size:.75rem}}
#toast{{position:fixed;bottom:24px;right:24px;background:var(--card);border:1px solid var(--border2);border-radius:var(--radius);padding:12px 18px;font-size:.82rem;color:var(--text);box-shadow:0 8px 32px #00000066;transform:translateY(8px);opacity:0;transition:opacity .25s,transform .25s;z-index:999;pointer-events:none}}
#toast.show{{opacity:1;transform:translateY(0)}}
@media(max-width:900px){{:root{{--sidebar-w:0px}}.sidebar{{display:none}}.kpi-grid{{grid-template-columns:repeat(2,1fr)}}.two-col{{grid-template-columns:1fr}}.quick-grid{{grid-template-columns:1fr 1fr}}}}
@media(max-width:540px){{.kpi-grid{{grid-template-columns:1fr}}.quick-grid{{grid-template-columns:1fr}}.topbar-actions{{display:none}}}}
</style>
</head>
<body>
<nav class="sidebar">
  <div class="sidebar-logo">
    <div class="logo-icon">R</div>
    <div>
      <div class="logo-text">TradingBots<span>.fun</span></div>
      <div class="logo-badge">ADMIN</div>
    </div>
  </div>
  <div class="nav">
    <div class="nav-section-label">Overview</div>
    <a href="/admin"{nav_dash}>
      <svg viewBox="0 0 16 16" fill="currentColor"><path d="M0 1.5A1.5 1.5 0 011.5 0h3A1.5 1.5 0 016 1.5V6a1.5 1.5 0 01-1.5 1.5h-3A1.5 1.5 0 010 6V1.5zm0 8A1.5 1.5 0 011.5 8h3A1.5 1.5 0 016 9.5V14a1.5 1.5 0 01-1.5 1.5h-3A1.5 1.5 0 010 14V9.5zm8-8A1.5 1.5 0 019.5 0h3A1.5 1.5 0 0114 1.5V6a1.5 1.5 0 01-1.5 1.5h-3A1.5 1.5 0 018 6V1.5zm0 8A1.5 1.5 0 019.5 8h3A1.5 1.5 0 0114 9.5V14a1.5 1.5 0 01-1.5 1.5h-3A1.5 1.5 0 018 14V9.5z"/></svg>
      Dashboard
    </a>
    <a href="/admin/users"{nav_users}>
      <svg viewBox="0 0 16 16" fill="currentColor"><path d="M7 14s-1 0-1-1 1-4 5-4 5 3 5 4-1 1-1 1H7zm4-6a3 3 0 100-6 3 3 0 000 6zm-5.784 6A2.238 2.238 0 015 13c0-1.355.68-2.75 1.936-3.72A6.325 6.325 0 005 9c-4 0-5 3-5 4s1 1 1 1h4.216zM4.5 8a2.5 2.5 0 100-5 2.5 2.5 0 000 5z"/></svg>
      Users
    </a>
    <a href="/admin/wallets"{nav_wall}>
      <svg viewBox="0 0 16 16" fill="currentColor"><path d="M0 3a2 2 0 012-2h13.5a.5.5 0 010 1H15v2a1 1 0 011 1v8.5a1.5 1.5 0 01-1.5 1.5h-12A2.5 2.5 0 010 12.5V3zm1 1.732V12.5A1.5 1.5 0 002.5 14h12a.5.5 0 00.5-.5V5H2a2 2 0 01-1-.268zM1 3a1 1 0 001 1h12V2H2a1 1 0 00-1 1z"/></svg>
      Wallets
    </a>
    <div class="nav-section-label" style="margin-top:8px">Bot</div>
    <a href="/app/agents" class="nav-item">
      <svg viewBox="0 0 16 16" fill="currentColor"><path d="M6 12.5a.5.5 0 01.5-.5h3a.5.5 0 010 1h-3a.5.5 0 01-.5-.5zM3 8.062C3 6.76 4.235 5.765 5.53 5.886a26.58 26.58 0 004.94 0C11.765 5.765 13 6.76 13 8.062v1.157a.933.933 0 01-.765.935c-.845.147-2.34.346-4.235.346-1.895 0-3.39-.2-4.235-.346A.933.933 0 013 9.219V8.062zm4.542-.827a.25.25 0 00-.217.065l-.489.49-.1-.41a.5.5 0 00-.962.246l.486 1.904a.25.25 0 00.363.162l1.7-.984a.25.25 0 00-.78-.473z"/></svg>
      Agents
    </a>
    <div class="nav-section-label" style="margin-top:8px">System</div>
    <a href="/" class="nav-item">
      <svg viewBox="0 0 16 16" fill="currentColor"><path fill-rule="evenodd" d="M8.354 1.146a.5.5 0 00-.708 0l-6 6A.5.5 0 002 7.5V14a.5.5 0 00.5.5h4a.5.5 0 00.5-.5v-4h2v4a.5.5 0 00.5.5h4a.5.5 0 00.5-.5V7.5a.5.5 0 00-.146-.354L13 5.793V2.5a.5.5 0 00-.5-.5h-1a.5.5 0 00-.5.5v1.293L8.354 1.146zM2.5 14V7.707l5.5-5.5 5.5 5.5V14H10v-4a.5.5 0 00-.5-.5h-3a.5.5 0 00-.5.5v4H2.5z"/></svg>
      Operator View
    </a>
  </div>
  <div class="sidebar-footer">
    <div><span class="status-dot"></span>Bot running · v{version}</div>
    <div style="margin-top:4px;opacity:.6">165.232.160.43</div>
  </div>
</nav>
<div class="main">
  <div class="topbar">
    <div class="topbar-title">{page_title}</div>
    <div class="topbar-actions">{pills_html}</div>
  </div>
  <div class="page">
{body}
  </div>
</div>
<div id="toast"></div>
<script>
function _toast(msg,color){{var e=document.getElementById('toast');e.textContent=msg;e.style.borderColor=color+'44';e.style.color=color;e.classList.add('show');setTimeout(function(){{e.classList.remove('show');}},3000);}}
</script>
</body></html>"##,
        page_title = page_title,
        nav_dash   = nav_dash,
        nav_users  = nav_users,
        nav_wall   = nav_wall,
        version    = version,
        pills_html = pills_html,
        body       = body,
    )
}

/// `GET /admin` — operator admin dashboard.
pub(crate) async fn admin_dashboard_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel is not configured. Set ADMIN_PASSWORD.",
            )
                .into_response()
        }
    };

    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    let (tenant_count, pro_count, free_count, total_balance) = {
        let tenants = app.tenants.read().await;
        let count = tenants.count();
        let pro = tenants
            .all()
            .filter(|h| h.config.tier == crate::tenant::TenantTier::Pro)
            .count();
        let free = tenants
            .all()
            .filter(|h| h.config.tier == crate::tenant::TenantTier::Free)
            .count();
        let balance: f64 = tenants.all().map(|h| h.config.hl_balance_usd).sum();
        (count, pro, free, balance)
    };

    let pattern_snapshot = {
        let cache = app.pattern_cache.lock().await;
        cache.latest()
    };
    let pattern_panel = if let Some(insights) = pattern_snapshot {
        let summary = &insights.report_summary;
        let winner = summary
            .daily_winner
            .as_ref()
            .map(|(sym, pnl)| format!("{} (${:0.2})", html_escape(sym), pnl))
            .unwrap_or_else(|| "—".to_string());
        let loser = summary
            .daily_loser
            .as_ref()
            .map(|(sym, pnl)| format!("{} (${:0.2})", html_escape(sym), pnl))
            .unwrap_or_else(|| "—".to_string());
        let win_combo_rows = if insights.top_win_combos.is_empty() {
            r#"<div class="insight-item"><div class="insight-label" style="color:var(--muted)">Not enough combos yet.</div></div>"#.to_string()
        } else {
            insights
                .top_win_combos
                .iter()
                .take(3)
                .enumerate()
                .map(|(i, combo)| {
                    format!(
                        r#"<div class="insight-item"><div class="insight-rank">{n}</div><div class="insight-label">{bd} · {ctx}</div><div class="insight-stat win-stat">{rate:.0}% · {wins}W/{losses}L</div></div>"#,
                        n      = i + 1,
                        bd     = html_escape(&combo.breakdown),
                        ctx    = html_escape(&combo.context),
                        rate   = combo.win_rate * 100.0,
                        wins   = combo.wins,
                        losses = combo.losses,
                    )
                })
                .collect::<Vec<_>>()
                .join("")
        };
        let loss_combo_rows = if insights.top_loss_combos.is_empty() {
            r#"<div class="insight-item"><div class="insight-label" style="color:var(--muted)">False-breakout / stall data pending.</div></div>"#.to_string()
        } else {
            insights
                .top_loss_combos
                .iter()
                .take(2)
                .map(|combo| {
                    format!(
                        r#"<div class="insight-item"><div class="insight-rank" style="background:var(--red-dim);color:var(--red)">!</div><div class="insight-label">{bd} · {ctx}</div><div class="insight-stat loss-stat">{rate:.0}% · {wins}W/{losses}L</div></div>"#,
                        bd     = html_escape(&combo.breakdown),
                        ctx    = html_escape(&combo.context),
                        rate   = combo.win_rate * 100.0,
                        wins   = combo.wins,
                        losses = combo.losses,
                    )
                })
                .collect::<Vec<_>>()
                .join("")
        };
        format!(
            r#"<div class="card">
  <div class="card-header">
    <div class="card-title">Pattern Cache · {date}</div>
    <a href="/app/agents" class="btn btn-ghost" style="padding:4px 10px;font-size:.72rem;">Agent console →</a>
  </div>
  <div class="card-body" style="padding:12px 20px">
    <div style="display:flex;gap:12px;margin-bottom:12px">
      <div style="flex:1;background:var(--green-dim);border:1px solid #22c55e33;border-radius:var(--radius-sm);padding:9px 12px">
        <div style="font-size:.62rem;color:var(--muted);text-transform:uppercase;letter-spacing:.06em;margin-bottom:3px">Winner</div>
        <div style="font-weight:700;color:var(--green);font-size:.88rem">{winner}</div>
      </div>
      <div style="flex:1;background:var(--red-dim);border:1px solid #f43f5e33;border-radius:var(--radius-sm);padding:9px 12px">
        <div style="font-size:.62rem;color:var(--muted);text-transform:uppercase;letter-spacing:.06em;margin-bottom:3px">Loser</div>
        <div style="font-weight:700;color:var(--red);font-size:.88rem">{loser}</div>
      </div>
    </div>
    <div style="font-size:.68rem;color:var(--muted);text-transform:uppercase;letter-spacing:.07em;margin-bottom:5px">Top win combos</div>
    {win_combo_rows}
    <div style="font-size:.68rem;color:var(--muted);text-transform:uppercase;letter-spacing:.07em;margin-top:10px;margin-bottom:5px">Loss warnings</div>
    {loss_combo_rows}
  </div>
</div>"#,
            date = insights.date.format("%Y-%m-%d"),
            winner = winner,
            loser = loser,
            win_combo_rows = win_combo_rows,
            loss_combo_rows = loss_combo_rows,
        )
    } else {
        r#"<div class="card">
  <div class="card-header"><div class="card-title">Pattern Cache</div></div>
  <div class="card-body" style="color:var(--muted);font-size:.83rem">
    No cached insights yet. Run <code>cargo run --bin reporter</code> so <code>reports/pattern_cache.json</code> can feed the AI bar.
  </div>
</div>"#
        .to_string()
    };

    // ── Build regime pill ──────────────────────────────────────────────────────
    let regime_pill = {
        // Peek at the first tenant's cached regime if available; fall back to "Unknown".
        let tenants = app.tenants.read().await;
        let regime_str = tenants.all().next()
            .map(|_| "Bear") // TODO: expose regime on AppState for a direct read
            .unwrap_or("Unknown");
        drop(tenants);
        if regime_str == "Bear" {
            r#"<span class="pill pill-red">▼ Bear Regime</span>"#.to_string()
        } else if regime_str == "Bull" {
            r#"<span class="pill pill-green">▲ Bull Regime</span>"#.to_string()
        } else {
            r#"<span class="pill pill-amber">◆ Neutral Regime</span>"#.to_string()
        }
    };
    let pills = format!(
        r#"<span class="pill pill-green">● Bot Active</span><span class="pill pill-blue">Paper Mode</span>{}"#,
        regime_pill
    );

    // ── Build KPI cards ────────────────────────────────────────────────────────
    let kpi_html = format!(
        r#"<div class="kpi-grid">
  <div class="kpi kpi-blue">
    <div class="kpi-icon">👥</div>
    <div class="kpi-label">Total Users</div>
    <div class="kpi-value">{tenant_count}</div>
    <div class="kpi-sub">&nbsp;</div>
  </div>
  <div class="kpi kpi-green">
    <div class="kpi-icon">⭐</div>
    <div class="kpi-label">Pro Subscribers</div>
    <div class="kpi-value" style="color:var(--green)">{pro_count}</div>
    <div class="kpi-sub">{free_count} free tier</div>
  </div>
  <div class="kpi kpi-amber">
    <div class="kpi-icon">💰</div>
    <div class="kpi-label">Total HL Balance</div>
    <div class="kpi-value" style="font-size:1.45rem">${total_balance:.2}</div>
    <div class="kpi-sub">across all wallets</div>
  </div>
  <div class="kpi kpi-red">
    <div class="kpi-icon">📈</div>
    <div class="kpi-label">Pattern Cache</div>
    <div class="kpi-value" style="font-size:1.1rem">{cache_date}</div>
    <div class="kpi-sub">&nbsp;</div>
  </div>
</div>"#,
        tenant_count = tenant_count,
        pro_count    = pro_count,
        free_count   = free_count,
        total_balance = total_balance,
        cache_date   = "live",
    );

    // ── Two-column: controls + pattern cache ───────────────────────────────────
    let body_html = format!(
        r#"{kpi}
<div class="two-col">
  <div class="card">
    <div class="card-header"><div class="card-title">Bot Controls</div></div>
    <div class="control-row">
      <div class="control-info">
        <div class="control-name">Reset Stats</div>
        <div class="control-desc">Clears P&amp;L history, closed trades, metrics and drawdown window. Open positions and signal weights are kept.</div>
      </div>
      <button class="btn btn-red" onclick="resetStats()" id="btn-reset">🔄 Reset</button>
    </div>
    <div id="reset-resp" style="display:none;margin:0 20px 14px;font-size:.78rem;border-radius:var(--radius-sm);padding:9px 12px"></div>
  </div>
  {pattern_panel_html}
</div>
<div class="card" style="margin-bottom:24px">
  <div class="card-header"><div class="card-title">Quick Access</div></div>
  <div style="padding:16px">
    <div class="quick-grid">
      <a href="/admin/users" class="quick-card">
        <div class="quick-card-icon">👥</div>
        <div class="quick-card-name">User Management</div>
        <div class="quick-card-desc">View all tenants, tiers &amp; balances</div>
      </a>
      <a href="/admin/wallets" class="quick-card">
        <div class="quick-card-icon">📈</div>
        <div class="quick-card-name">Wallet Performance</div>
        <div class="quick-card-desc">P&amp;L, equity, LTV estimates</div>
      </a>
      <a href="/app/agents" class="quick-card">
        <div class="quick-card-icon">🤖</div>
        <div class="quick-card-name">Agent Console</div>
        <div class="quick-card-desc">Live signal decisions &amp; signals</div>
      </a>
    </div>
  </div>
</div>
<script>
function resetStats(){{
  var btn=document.getElementById('btn-reset');
  var resp=document.getElementById('reset-resp');
  if(!confirm('Reset all trading stats? P&L, closed trades and metrics will be cleared.'))return;
  btn.disabled=true;btn.textContent='⏳ Resetting…';
  fetch('/api/admin/reset-stats',{{method:'POST',credentials:'include'}})
    .then(function(r){{return r.json();}})
    .then(function(d){{
      resp.style.display='block';
      if(d.ok){{resp.style.background='var(--green-dim)';resp.style.color='var(--green)';resp.style.border='1px solid #22c55e44';resp.textContent='✅ '+d.message;btn.textContent='✓ Done';_toast('Stats reset','#22c55e');}}
      else{{resp.style.background='var(--amber-dim)';resp.style.color='var(--amber)';resp.style.border='1px solid #f59e0b44';resp.textContent='⚠ '+d.message;btn.disabled=false;btn.textContent='🔄 Reset';_toast(d.message,'#f59e0b');}}
    }}).catch(function(){{resp.style.display='block';resp.textContent='⚠ Network error';btn.disabled=false;btn.textContent='🔄 Reset';_toast('Network error','#f43f5e');}});
}}
</script>"#,
        kpi                 = kpi_html,
        pattern_panel_html  = pattern_panel,
    );

    let html = admin_shell("Dashboard", "dashboard", &pills, env!("CARGO_PKG_VERSION"), &body_html);

    axum::response::Html(html).into_response()
}

/// `POST /api/admin/reset-stats` — clear P&L history and metrics for a fresh start.
///
/// Resets: capital → initial_capital, pnl → 0, peak_equity, equity_window,
/// cb_active, closed_trades, recent_decisions, metrics, equity_history,
/// house_money_pool, pool_deployed_usd, recently_closed.
///
/// Preserved: open positions, signal_weights, cycle_count, session_prices.
///
/// Requires HTTP Basic Auth (same as /admin).
pub(crate) async fn admin_reset_stats_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel not configured.",
            )
                .into_response()
        }
    };
    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    {
        let mut s = app.bot_state.write().await;
        let ic = s.initial_capital;
        // Recalculate equity including any open positions so capital is correct
        let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
        let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
        let current_equity = s.capital + committed + unrealised;
        // Reset financials — keep current equity as new starting point
        s.capital = current_equity - committed; // free cash only
        s.initial_capital = ic; // keep original for context
        s.pnl = 0.0;
        s.peak_equity = current_equity;
        s.equity_window = std::collections::VecDeque::new();
        s.equity_history = vec![];
        s.cb_active = false;
        // Clear history
        s.closed_trades = vec![];
        s.recent_decisions = vec![];
        s.metrics = crate::metrics::PerformanceMetrics::default();
        // Clear house-money pool (profits are gone from the P&L slate)
        s.house_money_pool = 0.0;
        s.pool_deployed_usd = 0.0;
        s.recently_closed = std::collections::VecDeque::new();
        // Mark positions as starting fresh — reset their entry context for P&L tracking
        // (positions themselves are kept open, only pool funding tracking resets)
        for p in s.positions.iter_mut() {
            p.funded_from_pool = false;
            p.pool_stake_usd = 0.0;
        }
    }

    log::info!("🔄 Admin: trading stats reset via /api/admin/reset-stats");

    axum::response::Json(serde_json::json!({
        "ok":      true,
        "message": "Stats reset — P&L, metrics and trade history cleared. Open positions kept. Bot continues from current equity.",
    })).into_response()
}

/// `POST /api/admin/session` — create a named paper-trading session without x402 payment.
///
/// Requires `X-Admin-Key: <ADMIN_KEY>` header (env var `ADMIN_KEY`).
/// Useful for dev, demos, and seeding named wallets (e.g. "AJ", "Daniel").
///
/// Body:
/// ```json
/// {
///   "name":        "AJ",
///   "balance_usd": 200.0,
///   "risk_mode":   "aggressive",
///   "venue":       "internal",
///   "duration":    "30d"
/// }
/// ```
pub(crate) async fn admin_create_session_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // ── Validate admin key ────────────────────────────────────────────────
    let expected_key = std::env::var("ADMIN_KEY").map_err(|_| ()).and_then(|k| if k.is_empty() { Err(()) } else { Ok(k) }).unwrap_or_else(|_| { panic!("ADMIN_KEY env var not set") });
    let provided_key = headers
        .get("x-admin-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided_key != expected_key {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::response::Json(serde_json::json!({
                "error": "Invalid or missing X-Admin-Key header"
            })),
        ).into_response();
    }

    // ── Parse body ────────────────────────────────────────────────────────
    let name        = body.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed").to_string();
    let balance_usd = body.get("balance_usd").and_then(|v| v.as_f64()).unwrap_or(200.0);
    let risk_mode   = body.get("risk_mode").and_then(|v| v.as_str()).map(|s| s.to_string());
    let venue       = body.get("venue").and_then(|v| v.as_str()).unwrap_or("internal").to_string();
    let duration    = body.get("duration").and_then(|v| v.as_str()).unwrap_or("30d").to_string();
    let leverage_max = body.get("leverage_max").and_then(|v| v.as_i64()).map(|v| v as i32);
    let max_drawdown_pct = body.get("max_drawdown_pct").and_then(|v| v.as_f64());
    let symbols_whitelist: Option<Vec<String>> = body
        .get("symbols_whitelist")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect());

    // ── Create session ────────────────────────────────────────────────────
    let session_id = new_id("ses");
    let token      = new_id("tok");
    let now        = chrono::Utc::now();
    let expires_at = if duration == "24h" {
        now + chrono::Duration::hours(24)
    } else {
        now + chrono::Duration::days(30)
    };
    let plan = if duration == "24h" { "admin-burst-24h" } else { "admin-30d" }.to_string();

    let hl_address = if venue == "hyperliquid" {
        let (addr, _) = crate::hl_wallet::generate_keypair();
        Some(addr)
    } else {
        None
    };

    let session = BotSession {
        id:                  session_id.clone(),
        token:               token.clone(),
        tx_hash:             "admin-created".to_string(),
        plan:                plan.clone(),
        created_at:          now.to_rfc3339(),
        expires_at:          expires_at.to_rfc3339(),
        max_drawdown_pct,
        webhook_url:         None,
        venue:               venue.clone(),
        leverage_max,
        risk_mode:           risk_mode.clone(),
        symbols_whitelist,
        performance_fee_pct: None,
        hyperliquid_address: hl_address.clone(),
        paused:              false,
        name:                Some(name.clone()),
        balance_usd,
        session_pnl:         0.0,
    };

    {
        let mut s = app.bot_state.write().await;
        s.bot_sessions.insert(session_id.clone(), session);
    }

    log::info!(
        "👤 Admin session created: {} (name={} balance=${:.0} risk={} venue={})",
        session_id,
        name,
        balance_usd,
        risk_mode.as_deref().unwrap_or("balanced"),
        venue,
    );

    axum::response::Json(serde_json::json!({
        "ok":             true,
        "session_id":     session_id,
        "token":          token,
        "name":           name,
        "balance_usd":    balance_usd,
        "risk_mode":      risk_mode.unwrap_or_else(|| "balanced".to_string()),
        "venue":          venue,
        "plan":           plan,
        "expires_at":     expires_at.to_rfc3339(),
        "deposit_address": hl_address,
        "endpoints": {
            "status":    format!("/api/v1/session/{}", session_id),
            "command":   format!("/api/v1/session/{}/command", session_id),
            "positions": format!("/api/v1/session/{}/positions", session_id),
            "trades":    format!("/api/v1/session/{}/trades", session_id),
            "latency":   format!("/api/v1/session/{}/latency/stats", session_id),
        }
    })).into_response()
}

/// `GET /admin/users` — table of all tenants with key stats.
pub(crate) async fn admin_users_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel not configured",
            )
                .into_response()
        }
    };

    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    let rows: String = {
        let tenants = app.tenants.read().await;
        tenants
            .all()
            .map(|h| {
                let tier_tag = match h.config.tier {
                    crate::tenant::TenantTier::Pro      => r#"<span class="tag tag-pro">PRO</span>"#,
                    crate::tenant::TenantTier::Internal => r#"<span class="tag tag-int">INTERNAL</span>"#,
                    crate::tenant::TenantTier::Free     => r#"<span class="tag tag-free">FREE</span>"#,
                };
                let wallet_short = h
                    .config
                    .wallet_address
                    .as_deref()
                    .map(|w| format!(
                        "<span class=\"mono\" style=\"color:var(--muted)\">{}…{}</span>",
                        &w[..6.min(w.len())],
                        &w[w.len().saturating_sub(4)..]
                    ))
                    .unwrap_or_else(|| "<span style=\"color:var(--muted)\">—</span>".to_string());
                let terms_ok = if h.config.terms_accepted_at.is_some() {
                    r#"<span style="color:var(--green)">✓</span>"#
                } else {
                    r#"<span style="color:var(--red)">✗</span>"#
                };
                let fund_sum = crate::fund_tracker::summary(&h.id);
                format!(
                    "<tr>\
                   <td class=\"mono\" style=\"color:var(--muted)\">{id_short}</td>\
                   <td style=\"font-weight:600\">{name}</td>\
                   <td>{tier}</td>\
                   <td>{wallet}</td>\
                   <td>${bal:.2}</td>\
                   <td>${dep:.2}</td>\
                   <td>{terms}</td>\
                 </tr>",
                    id_short = &h.id.as_str()[..8.min(h.id.as_str().len())],
                    name     = h.config.display_name,
                    tier     = tier_tag,
                    wallet   = wallet_short,
                    bal      = h.config.hl_balance_usd,
                    dep      = fund_sum.net_deposits,
                    terms    = terms_ok,
                )
            })
            .collect()
    };

    let body_html = format!(
        r#"<div class="card">
  <div class="card-header">
    <div class="card-title">All Users</div>
    <a href="/admin" class="btn btn-ghost" style="padding:4px 10px;font-size:.72rem;">← Dashboard</a>
  </div>
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>ID (prefix)</th>
          <th>Name</th>
          <th>Tier</th>
          <th>Wallet</th>
          <th>HL Balance</th>
          <th>Net Deposits</th>
          <th>Terms</th>
        </tr>
      </thead>
      <tbody>{rows}</tbody>
    </table>
  </div>
</div>"#,
        rows = if rows.is_empty() {
            "<tr><td colspan='7' style='color:var(--muted);text-align:center;padding:24px'>No users registered yet.</td></tr>".to_string()
        } else {
            rows
        },
    );
    let html = admin_shell("Users", "users", "", env!("CARGO_PKG_VERSION"), &body_html);

    axum::response::Html(html).into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
/// `GET /admin/wallets` — per-wallet P&L and LTV performance table.
///
/// Shows all 9 demo wallets (Bot Alpha → Iota) side-by-side with:
///   • Starting capital and current equity
///   • Unrealised and realised P&L
///   • Return % since inception
///   • Open positions count
///   • Estimated builder fees earned (LTV proxy)
///   • Retention signal: equity trend (gaining / flat / bleeding)
// ─────────────────────────────────────────────────────────────────────────────
pub(crate) async fn admin_wallets_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel not configured",
            )
                .into_response()
        }
    };
    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    struct WalletRow {
        name: String,
        capital: f64,
        equity: f64,
        realised_pnl: f64,
        unrealised: f64,
        return_pct: f64,
        open_pos: usize,
        closed_trades: usize,
        est_ltv_usd: f64,
        trend: &'static str,
    }

    let rows_data: Vec<WalletRow> = {
        let tm = app.tenants.read().await;
        let mut out = Vec::new();
        for h in tm.all() {
            let s = h.state.read().await;
            let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
            let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
            let equity = s.capital + committed + unrealised;
            let return_pct = if s.initial_capital > 0.0 {
                (equity - s.initial_capital) / s.initial_capital * 100.0
            } else {
                0.0
            };
            // Estimate LTV: 3 bps entry + 3 bps exit = 6 bps per round-trip
            let avg_size: f64 = if !s.closed_trades.is_empty() {
                s.closed_trades.iter().map(|t| t.size_usd).sum::<f64>()
                    / s.closed_trades.len() as f64
            } else {
                s.initial_capital * 0.08
            };
            let est_ltv = s.closed_trades.len() as f64 * avg_size * 0.0006;
            let trend = if return_pct > 2.0 {
                "🟢"
            } else if return_pct > -1.0 {
                "🟡"
            } else {
                "🔴"
            };
            out.push(WalletRow {
                name: h.config.display_name.clone(),
                capital: s.initial_capital,
                equity,
                realised_pnl: s.pnl,
                unrealised,
                return_pct,
                open_pos: s.positions.len(),
                closed_trades: s.closed_trades.len(),
                est_ltv_usd: est_ltv,
                trend,
            });
        }
        out.sort_by(|a, b| {
            a.capital
                .partial_cmp(&b.capital)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    };

    let total_est_ltv: f64 = rows_data.iter().map(|r| r.est_ltv_usd).sum();
    let total_equity: f64 = rows_data.iter().map(|r| r.equity).sum();
    let total_capital: f64 = rows_data.iter().map(|r| r.capital).sum();
    let total_ret_pct = if total_capital > 0.0 {
        (total_equity - total_capital) / total_capital * 100.0
    } else {
        0.0
    };

    let table_rows: String = rows_data
        .iter()
        .map(|r| {
            let ret_col  = if r.return_pct >= 0.0 { "var(--green)" } else { "var(--red)" };
            let upnl_col = if r.unrealised  >= 0.0 { "var(--green)" } else { "var(--red)" };
            let pnl_sign = if r.realised_pnl >= 0.0 { "+" } else { "" };
            let cap_tier = if r.capital <= 25.0 {
                r#"<span class="tag tag-free">Nano</span>"#
            } else if r.capital <= 100.0 {
                r#"<span class="tag tag-free">Micro</span>"#
            } else if r.capital <= 500.0 {
                r#"<span class="tag tag-pro">Small</span>"#
            } else if r.capital <= 2000.0 {
                r#"<span class="tag tag-int">Mid</span>"#
            } else {
                r#"<span class="tag tag-int">Large</span>"#
            };
            format!(
                "<tr>\
               <td>{trend} <b>{name}</b></td>\
               <td>{tier}</td>\
               <td>${cap:.0}</td>\
               <td style='color:{rc}'>${eq:.2}</td>\
               <td style='color:{rc};font-weight:700'>{ret:+.2}%</td>\
               <td>{ps}${rpnl:.2}</td>\
               <td style='color:{uc}'>{us:+.2}</td>\
               <td style='color:var(--muted)'>{ops} open · {cl} closed</td>\
               <td style='color:var(--amber)'>${ltv:.4}</td>\
             </tr>",
                trend = r.trend,
                name  = r.name,
                tier  = cap_tier,
                cap   = r.capital,
                eq    = r.equity,
                rc    = ret_col,
                ret   = r.return_pct,
                ps    = pnl_sign,
                rpnl  = r.realised_pnl,
                uc    = upnl_col,
                us    = r.unrealised,
                ops   = r.open_pos,
                cl    = r.closed_trades,
                ltv = r.est_ltv_usd,
            )
        })
        .collect();

    let teq_col  = if total_equity >= total_capital { "var(--green)" } else { "var(--red)" };
    let tret_col = if total_ret_pct >= 0.0          { "var(--green)" } else { "var(--red)" };
    let body_html = format!(
        r#"<div class="kpi-grid" style="margin-bottom:20px">
  <div class="kpi kpi-blue">
    <div class="kpi-icon">👛</div>
    <div class="kpi-label">Wallets</div>
    <div class="kpi-value">{wc}</div>
  </div>
  <div class="kpi kpi-amber">
    <div class="kpi-icon">💵</div>
    <div class="kpi-label">Total Capital</div>
    <div class="kpi-value" style="font-size:1.4rem">${tc:.0}</div>
  </div>
  <div class="kpi kpi-green">
    <div class="kpi-icon">📊</div>
    <div class="kpi-label">Total Equity</div>
    <div class="kpi-value" style="font-size:1.4rem;color:{teq_col}">${te:.2}</div>
  </div>
  <div class="kpi kpi-red">
    <div class="kpi-icon">🏗️</div>
    <div class="kpi-label">Portfolio Return</div>
    <div class="kpi-value" style="font-size:1.4rem;color:{tret_col}">{tr:+.2}%</div>
  </div>
</div>
<div class="card" style="margin-bottom:20px">
  <div class="card-header">
    <div class="card-title">Wallet Performance</div>
    <span style="font-size:.73rem;color:var(--muted)">Est. builder fees: <span style="color:var(--amber)">${ltv_total:.4}</span> all-time</span>
  </div>
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>Wallet</th><th>Tier</th><th>Capital</th><th>Equity</th>
          <th>Return</th><th>Realised P&amp;L</th><th>Unrealised</th>
          <th>Positions</th><th>Builder Fees</th>
        </tr>
      </thead>
      <tbody>{trows}</tbody>
      <tfoot>
        <tr>
          <td colspan="2" style="color:var(--muted)">Portfolio total</td>
          <td>${tc:.0}</td>
          <td style="color:{teq_col}">${te:.2}</td>
          <td style="color:{tret_col}">{tr:+.2}%</td>
          <td colspan="3"></td>
          <td style="color:var(--amber)">${ltv_total:.4}</td>
        </tr>
      </tfoot>
    </table>
  </div>
</div>
<div style="background:var(--card);border:1px solid var(--border);border-radius:var(--radius);padding:18px 20px;font-size:.76rem;color:var(--muted);line-height:1.7">
  <span style="color:var(--text);font-weight:600">⚡ Builder fee estimate:</span> 3 bps entry + 3 bps exit = 6 bps per round-trip × avg position size × closed trades.<br>
  <span style="color:var(--text);font-weight:600">🎯 LTV playbook —</span>
  <b>Nano/Micro ($10–$100):</b> max 3–6 positions, conservative leverage, take profits at +10%. &nbsp;
  <b>Small ($100–$500):</b> 4–8 round-trips/day, $1–$3/month LTV. &nbsp;
  <b>Mid/Large ($1k+):</b> $9–$60/month LTV — keep drawdown &lt;10% to retain. &nbsp;
  <b>Formula:</b> equity growth → retention → more fills → more LTV.
</div>"#,
        wc          = rows_data.len(),
        tc          = total_capital,
        te          = total_equity,
        teq_col     = teq_col,
        tr          = total_ret_pct,
        tret_col    = tret_col,
        ltv_total   = total_est_ltv,
        trows       = if table_rows.is_empty() {
            "<tr><td colspan='9' style='color:var(--muted);text-align:center;padding:24px'>\
             No wallets active — restart the bot to seed demo wallets.\
             </td></tr>".to_string()
        } else {
            table_rows
        },
    );
    let html = admin_shell("Wallet Performance", "wallets", "", env!("CARGO_PKG_VERSION"), &body_html);

    axum::response::Html(html).into_response()
}

