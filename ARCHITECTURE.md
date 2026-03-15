# RedRobot Architecture — Product Lifecycle & Scaling Design

*Living document. Updated after the system was confirmed working in production.*

---

## 1. Reality Check — What's Already Built

Before designing anything new, the current codebase is further along than it appears on the surface:

| Module | Status | Notes |
|---|---|---|
| `privy.rs` | ✅ Built, **dormant** | Full ES256 JWT verification, JWKS caching, HMAC session cookies |
| Login page (`/login`) | ✅ Built, **dormant** | Loads `@privy-io/js-sdk-core` from CDN, POSTs token to `/auth/session` |
| `auth_session_handler` | ✅ Built | Verifies Privy JWT, auto-registers tenant, issues `rr_session` cookie |
| `tenant.rs` | ✅ Built | UUID IDs, `privy_did` field, `wallet_address` field, `register_or_get_by_privy_did()` |
| `stripe.rs` | ✅ Built | Checkout session, webhook verification, tier upgrade/downgrade |
| `fund_tracker.rs` | ✅ Built | Deposit/withdrawal event detection and logging |
| `db.rs` | ⚠️ No-op stub | Compiles but does nothing — SQLite goes here |
| Admin dashboard | ✅ Built | Single-tenant analytics, trade history |
| Terms wall | ✅ Built | `/app/terms` gate before dashboard access |

**Why Privy appears missing:** `PRIVY_APP_ID` env var is not set. When it's `None`, the entire auth layer is bypassed — the server runs in single-operator mode. Setting `PRIVY_APP_ID=your_app_id` in the VPS environment is literally the only change needed to activate multi-tenant login. Everything else is already wired.

---

## 2. The Three Identity Keys — Never Confuse Them

```
TenantId (UUID)         ← internal primary key, never shown to users
privy_did               ← "did:privy:clxxxxxxxxx", links across devices/browsers
wallet_address (0x…)    ← what users recognise; what Hyperliquid uses
```

**tenant_id should NOT be the wallet address.** Reasons:
- 42-char hex is ugly in logs and makes join keys slow
- Users can rotate wallets (key compromise, hardware wallet migration)
- Some users will get a Privy embedded wallet and never know the address
- UUID is generated at `TenantId::new()` already — this is correct

**What to show in the dashboard header:**
- If wallet is linked: `0x1234…abcd` (first 6 + last 4 chars) — standard DeFi convention
- If no wallet yet: user's first name or email (from Privy claims — add `email` and `name` fields to `PrivyClaims`)
- Add a "Link Wallet" CTA badge in the header when `wallet_address` is `None`
- Never expose the UUID or the `privy_did` to users

```
Header mockup (wallet linked):   [🟢 RedRobot]  0x1234…abcd  [$12,450.33 AUM]  [Logout]
Header mockup (wallet missing):  [🟢 RedRobot]  daniel@…  [⚠ Link Wallet]  [Logout]
```

---

## 3. Game Theory of Usage Patterns → SQLite Schema

### Actor behaviour model

| Actor | Frequency | Pattern | Dominant operation |
|---|---|---|---|
| Trading bot | Every 30s | Reads all positions, writes equity snapshots, closes/opens positions | Batch write tx |
| Tenant user | Every 35s (auto-refresh) | Reads own positions, equity, metrics | Narrow read (WHERE tenant_id = ?) |
| Admin | ~10×/day | Cross-tenant aggregations, TVL graph, fee revenue | Expensive read — must be pre-aggregated |
| Privy auth | On login | Create or retrieve tenant | Rare, negligible |
| Stripe webhook | On payment | Upgrade/downgrade tier | Rare, negligible |
| Landing page visitor | Variable | Reads TVL graph (public endpoint) | Read from `aum_snapshots`, no JOIN |

### Derived schema constraints

1. **Hot write path:** equity snapshot writes dominate. At 500 tenants: 500 writes/30s = ~17 writes/sec. SQLite WAL mode handles ~10,000 writes/sec. Not a problem — but **always batch these in a single transaction** per trading cycle.

2. **Critical read pattern:** every dashboard load is `WHERE tenant_id = ?` — composite indexes on `(tenant_id, ts)` and `(tenant_id, symbol)` are mandatory.

3. **Admin aggregations must be pre-computed.** Never run `SELECT SUM(equity) FROM equity_snapshots` at query time across 500 tenants × 288 rows = 144,000 rows per page load. Instead, the trading loop writes one `aum_snapshots` row per cycle — admin queries are then a trivial single-table scan.

4. **Equity snapshots need pruning.** 500 tenants × 288 rows each = 144,000 rows just for the sparkline window. With 7-day retention at 30s intervals = 20,160 rows per tenant = 10 million rows. Use a dedicated table with aggressive pruning (DELETE WHERE ts < now - 7 days), run as a background task hourly.

5. **Signal weights are per-tenant but tiny.** One JSON blob per tenant. Use a simple key-value table — no need to normalise the weights into columns.

### SQLite schema

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = NORMAL;   -- safe with WAL, faster than FULL

-- ── Tenants ──────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS tenants (
  id               TEXT    PRIMARY KEY,   -- UUID (TenantId)
  privy_did        TEXT    UNIQUE,        -- "did:privy:cl…"
  wallet_address   TEXT    UNIQUE,        -- "0x…", nullable until linked
  display_name     TEXT,                  -- email or Privy display name
  tier             TEXT    NOT NULL DEFAULT 'Free',  -- Free | Pro | Internal
  initial_capital  REAL    NOT NULL DEFAULT 0,
  created_at       INTEGER NOT NULL,      -- unix seconds
  terms_accepted   INTEGER,               -- unix seconds, NULL = not accepted
  stripe_customer  TEXT,                  -- Stripe customer ID
  trial_ends_at    INTEGER                -- unix seconds, NULL = no trial
);
CREATE INDEX IF NOT EXISTS idx_tenants_privy  ON tenants(privy_did);
CREATE INDEX IF NOT EXISTS idx_tenants_wallet ON tenants(wallet_address);

-- ── Equity snapshots (sparkline data) ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS equity_snapshots (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  tenant_id TEXT    NOT NULL REFERENCES tenants(id),
  ts        INTEGER NOT NULL,    -- unix seconds
  equity    REAL    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_equity_tenant_ts ON equity_snapshots(tenant_id, ts);
-- Pruned hourly: DELETE FROM equity_snapshots WHERE ts < unixepoch() - 604800

-- ── Open positions ────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS positions (
  id               TEXT PRIMARY KEY,   -- "{tenant_id}:{symbol}"
  tenant_id        TEXT NOT NULL REFERENCES tenants(id),
  symbol           TEXT NOT NULL,
  side             TEXT NOT NULL,      -- LONG | SHORT
  entry_price      REAL,
  size_usd         REAL,
  notional_usd     REAL,
  leverage         REAL,
  stop_price       REAL,
  tp_price         REAL,
  opened_at        INTEGER,
  dca_count        INTEGER DEFAULT 0,
  tranche          INTEGER DEFAULT 0,
  cycles_held      INTEGER DEFAULT 0,
  signal_contrib   TEXT               -- JSON blob: SignalContribution
);
CREATE INDEX IF NOT EXISTS idx_positions_tenant ON positions(tenant_id);

-- ── Closed trades ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS closed_trades (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  tenant_id        TEXT    NOT NULL REFERENCES tenants(id),
  symbol           TEXT    NOT NULL,
  side             TEXT    NOT NULL,
  entry_price      REAL,
  exit_price       REAL,
  size_usd         REAL,
  pnl_usd          REAL,
  pnl_pct          REAL,
  r_multiple       REAL,
  fees_usd         REAL    DEFAULT 0,
  opened_at        INTEGER,
  closed_at        INTEGER NOT NULL,
  close_reason     TEXT,              -- HIT_STOP | HIT_TP | TRAILING | MANUAL
  signal_contrib   TEXT               -- JSON blob for post-close learner replay
);
CREATE INDEX IF NOT EXISTS idx_trades_tenant_ts ON closed_trades(tenant_id, closed_at);
CREATE INDEX IF NOT EXISTS idx_trades_ts         ON closed_trades(closed_at);  -- admin cross-tenant

-- ── Fund events (deposits / withdrawals / performance fees) ───────────────────
CREATE TABLE IF NOT EXISTS fund_events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  tenant_id     TEXT    NOT NULL REFERENCES tenants(id),
  ts            INTEGER NOT NULL,
  event_type    TEXT    NOT NULL,   -- DEPOSIT | WITHDRAWAL | PERFORMANCE_FEE | REFERRAL
  amount_usd    REAL    NOT NULL,
  balance_after REAL    NOT NULL,
  notes         TEXT
);
CREATE INDEX IF NOT EXISTS idx_fund_tenant_ts ON fund_events(tenant_id, ts);
CREATE INDEX IF NOT EXISTS idx_fund_ts         ON fund_events(ts);             -- admin

-- ── Per-tenant signal weights ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS signal_weights (
  tenant_id    TEXT    PRIMARY KEY REFERENCES tenants(id),
  weights_json TEXT    NOT NULL,
  updated_at   INTEGER NOT NULL
);

-- ── Pre-aggregated AUM snapshots (admin dashboard + TVL hero graph) ───────────
CREATE TABLE IF NOT EXISTS aum_snapshots (
  id                   INTEGER PRIMARY KEY AUTOINCREMENT,
  ts                   INTEGER NOT NULL,
  total_aum            REAL    NOT NULL,   -- sum of all tenant equity
  deposited_capital    REAL    NOT NULL,   -- sum of all initial_capital
  total_pnl            REAL    NOT NULL,   -- total_aum - deposited_capital
  pnl_pct              REAL    NOT NULL,   -- total_pnl / deposited_capital * 100
  active_tenant_count  INTEGER NOT NULL,
  open_position_count  INTEGER NOT NULL,
  total_trades_today   INTEGER NOT NULL,
  win_rate_today       REAL
);
CREATE INDEX IF NOT EXISTS idx_aum_ts ON aum_snapshots(ts);
```

### What moves from BotState to SQLite

```
BotState.positions          → positions table (keyed by tenant_id + symbol)
BotState.equity_history     → equity_snapshots table
BotState.closed_trades      → closed_trades table
BotState.signal_weights     → signal_weights table (per-tenant)
fund_tracker flat files      → fund_events table
```

BotState keeps in-memory caches of the above for performance — SQLite is the source of truth, written every cycle. On restart, load from SQLite.

---

## 4. Privy Activation — What Actually Needs to Happen

The backend is ready. The only pending items:

### Environment variable
```bash
# Add to VPS /etc/environment or systemd service file:
PRIVY_APP_ID=your_privy_app_id_here
SESSION_SECRET=64_random_hex_chars  # already required
```

### Privy dashboard setup (privy.io)
1. Create app → get App ID
2. Configure allowed login methods: Email, Google, MetaMask, WalletConnect
3. Set redirect URLs: `http://165.232.160.43:3000` (and future domain)
4. Note: Privy embedded wallets = users who sign up with email get a custodial wallet automatically — they never need to "have crypto" to use RedRobot

### Frontend: what the login page currently does
The `/login` page already loads `@privy-io/js-sdk-core` from CDN and injects the App ID into the JavaScript at render time. The flow:
1. User clicks "Sign in with Privy" → Privy modal opens
2. User authenticates (email OTP, Google, or connects wallet)
3. SDK returns `accessToken` (ES256 JWT)
4. Page POSTs to `POST /auth/session` with the token
5. Server verifies JWT via Privy JWKS, creates/retrieves tenant, sets `rr_session` cookie
6. Redirect to `/app/dashboard`

No code changes needed for this flow — just the env var.

### What IS missing: wallet linking after login
After Privy login, new users have no `wallet_address`. You need:
- A `/app/setup` page that prompts the user to connect their Hyperliquid wallet
- `PATCH /api/tenant/wallet` endpoint to write `wallet_address` into the DB
- The header should show a "Link Wallet" warning until this is done
- The bot should not trade for the tenant until `wallet_address` is set and capital is confirmed

---

## 5. Admin Account — Two-Layer Architecture

### Layer 1: Operator admin (you)
- Remains password-protected (`ADMIN_PASSWORD` env var)
- Routes: `/admin`, `/admin/tenants`, `/admin/fund`
- The existing admin panel is the right foundation — needs analytics expansion

### Layer 2: Admin analytics additions
The admin dashboard needs a second tab or section with:

**TVL / AUM panel**
- Total AUM (sum equity across all tenants), with sparkline from `aum_snapshots`
- Breakdown: deposited capital vs. trading gains vs. fees extracted
- Per-tier breakdown (Free vs. Pro vs. Internal)

**Portfolio analytics**
- Aggregate win rate (all tenants, all time)
- Most traded symbols
- Signal weight distribution across all tenants (are weights converging?)
- Average R-multiple per close reason

**Revenue tracking**
- Performance fees collected per tenant per month
- Stripe MRR
- Churn indicators (tenants who haven't opened a position in 7 days)

**Risk panel**
- Largest single-tenant exposure as % of total AUM
- Tenants in drawdown > 10% of initial capital
- Daily circuit-breaker triggers per tenant

### Admin TVL endpoint for landing page
```
GET /api/public/tvl    →  { ts, total_aum, pnl_pct, tenant_count, points: [(ts, aum)] }
```
No auth required. Returns pre-aggregated data only — no individual tenant details.
Rate-limited to 10 req/min to prevent scraping.

---

## 6. TVL Graph as Landing Page Hero

The equity sparkline SVG generator in `web_dashboard.rs` can be reused nearly verbatim. For the public TVL graph:

- Query `SELECT ts, total_aum FROM aum_snapshots WHERE ts > ? ORDER BY ts` — one fast index scan
- Feed into the same `to_y()` / `pts` / polyline / polygon fill / circle dot pattern
- Anchor the baseline to `deposited_capital` (not `initial_capital`) so the chart shows real alpha vs. just capital growth from deposits
- Show two lines if you want: total AUM (top line, includes deposits) and pure PnL curve (AUM minus deposits)

**Landing page hero section:**
```
         ┌─────────────────────────────────────────────────────┐
         │  [RedRobot logo]                                     │
         │                                                      │
         │  $247,312 AUM · +18.4% since inception              │
         │                                                      │
         │  [~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~▲]       │  ← live SVG
         │   Oct      Nov      Dec      Jan      Feb  Now       │
         │                                                      │
         │  23 traders · 847 closed trades · 64% win rate      │
         │                                                      │
         │  [Start free trial]    [Connect wallet]              │
         └─────────────────────────────────────────────────────┘
```

The SVG can be fetched client-side (JavaScript `fetch('/api/public/tvl')`) or server-side rendered into the HTML. Because the landing page is a marketing page (static or lightweight), the latter is cleaner — bake the SVG into the page HTML on each request, no JS required for the graph.

---

## 7. Full Application Lifecycle Roadmap

### Phase 1 — Single operator, production live (NOW ✅)
Bot is running on the VPS. Single-operator mode (no `PRIVY_APP_ID`). You are the only user.
**Goal:** accumulate real trade data, tune signal weights, validate the loop end-to-end.
**Exit criteria:** 50+ closed trades, signal weights have visibly shifted from defaults.

### Phase 2 — SQLite persistence (2–3 weeks)
Replace `db.rs` no-op stub with `rusqlite` (or `sqlx` + SQLite). Migrate:
- `BotState.positions` → `positions` table
- `equity_history` push → `equity_snapshots` write (batched per cycle)
- Closed trade logging → `closed_trades` table
- Signal weights → `signal_weights` table
- Fund events → `fund_events` table
- Add `aum_snapshots` write at end of each trading cycle

Still single-operator mode. No user-facing changes.
**Exit criteria:** bot can restart and resume all positions from DB. AUM graph has 7 days of data.

### Phase 3 — Admin analytics + TVL hero (parallel with Phase 2)
- Add TVL sparkline to admin dashboard using `aum_snapshots`
- Add `/api/public/tvl` endpoint
- Build landing page HTML (separate static file or new route in web_dashboard.rs)
- The landing page embeds the TVL graph as social proof

**Exit criteria:** landing page live at the domain, TVL graph looks compelling.

### Phase 4 — Privy activation + multi-tenant isolation (3–4 weeks)
- Set `PRIVY_APP_ID` env var
- Migrate `BotState.positions` to `HashMap<TenantId, HashMap<String, PaperPosition>>`
- Add wallet-linking flow (`/app/setup` page + `PATCH /api/tenant/wallet`)
- Add wallet address display in dashboard header
- Each tenant sees only their own data; bot trades per-tenant capital independently

**Exit criteria:** first external user can sign up, link wallet, and see their own dashboard.

### Phase 5 — Stripe billing activation (1 week, after Phase 4)
- Set `STRIPE_SECRET_KEY` + `STRIPE_WEBHOOK_SECRET` env vars
- Add "Upgrade to Pro" button in dashboard pointing to `/billing/checkout`
- Set `STRIPE_WEBHOOK_URL` in Stripe dashboard to `https://yourdomain.com/webhooks/stripe`
- Implement performance fee calculation from `fund_events` (e.g., 20% of profits above high-water mark, charged monthly)

**Exit criteria:** first paying user, first automated fee collection.

### Phase 6 — Per-tenant learning (after 10+ active users)
- Fork global `signal_weights.json` into per-tenant `signal_weights` DB rows
- On new tenant creation: initialise from global weights (Bayesian prior)
- After 20+ closed trades: weights diverge toward tenant-specific behaviour
- Global weights = rolling average across all tenants' weights (background task)

**Exit criteria:** two tenants' weights are measurably different and both improving.

### Phase 7 — Scale (when approaching 100+ tenants)
- Profile SQLite write latency — still likely fine
- If equity snapshot writes become a bottleneck: write to in-memory `BTreeMap` first, flush to SQLite every 5 minutes
- If read latency spikes: add connection pool (`r2d2` + `rusqlite`, 4–8 connections in WAL mode)
- PostgreSQL migration: `sqlx` makes this a connection-string change if schema is clean
- Consider WebSockets for dashboard push instead of 35s HTTP polling (eliminates 14 req/sec at 500 users)

---

## 8. The Wallet Address Question — Dashboard Header

Current header: `RedRobot AI · [timestamp]`

Proposed header (multi-tenant mode):
```
Left:   [🟢] RedRobot                ← existing gradient logo
Centre: [wallet badge]               ← 0x1234…abcd in monospace, dim colour
Right:  [Logout] [tier badge]        ← "Pro" or "Free" pill
```

The wallet address is the natural DeFi identity marker — users are proud of their wallet address the same way they'd be proud of a username. Showing it confirms "this session is yours and your funds are in your custody." It also serves as a security check (users notice if it's wrong).

Implementation: fetch from `s.wallet_address` in the `BotState` and truncate in the Rust template:
```rust
let wallet_display = config.wallet_address.as_deref()
    .map(|w| format!("{}…{}", &w[..6], &w[w.len()-4..]))
    .unwrap_or_else(|| "⚠ Wallet not linked".to_string());
```

---

## 9. AI Provider — Multi-Provider Architecture

The bot routes trade-analysis prompts through a single `query_ai()` function.
The backend is selected entirely via environment variables — no recompile needed.

### Provider routing table

| `AI_PROVIDER` | API endpoint | Default model | Best for |
|---|---|---|---|
| `claude` *(default)* | `api.anthropic.com/v1/messages` | `claude-haiku-4-5-20251001` | Quality reasoning, MCP integration |
| `openai` | `api.openai.com/v1/chat/completions` | `gpt-4o-mini` | Familiar tooling, function calling |
| `xai` | `api.x.ai/v1/chat/completions` | `grok-2` | Real-time market sentiment (tweets) |
| `openrouter` | `openrouter.ai/api/v1/chat/completions` | `openai/gpt-4o-mini` | Cost arbitrage across 200+ models |
| `ollama` | `{OLLAMA_BASE_URL}/api/generate` | `llama3.2` | Air-gapped / zero API cost |

### Environment variables

```
AI_PROVIDER=claude          # which backend (see table above)
AI_API_KEY=sk-ant-...       # API key for cloud providers (not needed for ollama)
AI_MODEL=claude-haiku-4-5-20251001  # model string for chosen provider
OLLAMA_BASE_URL=http://OLLAMA_DROPLET_IP:11434  # ollama only
```

### The Ollama two-droplet rule

**Ollama MUST NOT run on the trading-bot VPS.**

`llama3.2` (the smallest usable model) occupies ~4-6 GB of RAM at inference
time. A DigitalOcean droplet running the Rust bot, PostgreSQL, and Ollama
simultaneously will OOM-kill one of the three processes — almost certainly
Ollama, which then takes the bot's AI analysis loop down with it.

The correct topology:

```
┌─────────────────────────────┐      ┌─────────────────────────────┐
│  Trading-bot VPS             │      │  Ollama droplet (separate)   │
│  4 GB RAM recommended       │      │  8 GB RAM recommended        │
│  • hedgebot (Rust)          │      │  • ollama serve              │
│  • PostgreSQL 16            │─────▶│  • llama3.2 / mistral / etc  │
│  • Claude MCP server (Node) │      │  • port 11434 (firewall:     │
│                             │      │    open to bot VPS IP only)  │
└─────────────────────────────┘      └─────────────────────────────┘
```

Provision the Ollama droplet:
```bash
export OLLAMA_IP=<new-droplet-ip>
./deploy.sh --provision-ollama
# Automatically: installs Ollama, pulls llama3.2, restricts port 11434
# to the trading-bot VPS IP via ufw, writes OLLAMA_BASE_URL into
# /etc/environment on the trading-bot VPS, restarts hedgebot.
```

### Switching providers

Change one env var on the trading-bot VPS and restart:
```bash
# Switch from Claude to OpenRouter (cheapest per-token for high volume)
grep -v "^AI_" /etc/environment > /tmp/env && mv /tmp/env /etc/environment
echo "AI_PROVIDER=openrouter"               >> /etc/environment
echo "AI_API_KEY=sk-or-v1-..."             >> /etc/environment
echo "AI_MODEL=openai/gpt-4o-mini"         >> /etc/environment
systemctl restart hedgebot
```

---

## 10. Summary of Immediate Next Steps (Priority Order)

1. **Push commit** — run `git push origin master` from your local Mac terminal. The VM has no internet access so push must come from your machine.

2. **Provision trading-bot VPS** — `./deploy.sh --provision` installs PostgreSQL 16, the MCP server, and sets up the database. Does NOT install Ollama.

3. **Set `PRIVY_APP_ID`** — one env var activates the entire existing multi-tenant auth flow. Test with your own account first.

4. **Wallet display in header** — show `0x…` in the dashboard header. One line of template code.

5. **Landing page** — new route `/` (or separate static page) with the TVL graph SVG and CTA. This is the marketing and user acquisition engine.

6. *(Optional)* **Provision Ollama droplet** — only if you want local/private LLM inference. `export OLLAMA_IP=<ip> && ./deploy.sh --provision-ollama`. Otherwise keep `AI_PROVIDER=claude` and use the API.
