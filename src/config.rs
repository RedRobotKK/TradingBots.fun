use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Mode {
    Paper,
    Testnet,
    Mainnet,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub mode: Mode,
    pub trading_symbol: String,        // legacy single-symbol field
    pub trading_symbols: Vec<String>,  // "ALL" or comma-separated list
    pub initial_capital: f64,
    pub max_position_pct: f64,
    pub max_leverage: f64,
    pub daily_loss_limit: f64,
    pub min_health_factor: f64,

    // API Keys – optional for paper-trading
    pub binance_api_key: Option<String>,
    pub hyperliquid_key: Option<String>,
    pub hyperliquid_secret: Option<String>,
    pub lunarcrush_api_key: String,
    pub anthropic_api_key: Option<String>,

    // Hyperliquid wallet — required for testnet/mainnet
    /// Ethereum-style wallet address (0x…) — used for clearinghouseState queries
    /// and as the signer identity for all order submissions.
    pub hyperliquid_wallet_address: Option<String>,

    // Revenue — builder code embedded in every HL order
    /// Hyperliquid builder address (0x…).  When set, every order routed through
    /// this bot embeds the builder code so the platform earns the builder fee.
    /// Leave unset in paper mode; set on testnet to validate the code path.
    pub builder_code: Option<String>,

    /// Builder fee in basis points for the operator's own trading account.
    /// 1 = Pro reward (operator is always Internal/Pro).
    /// Overridden per-tenant at runtime via `TenantConfig::builder_fee_bps()`.
    /// Set via `BUILDER_FEE_BPS` env var; defaults to 1 if unset.
    /// HL maximum is 3 bps — values above 3 are clamped before submission.
    pub builder_fee_bps: u32,

    // Stripe — subscription billing
    /// Stripe secret API key (sk_live_… / sk_test_…).
    pub stripe_secret_key:      Option<String>,
    /// Stripe webhook signing secret (whsec_…) — verifies webhook authenticity.
    pub stripe_webhook_secret:  Option<String>,
    /// Stripe Price ID for the Pro subscription ($19.99/month).
    pub stripe_price_id:        Option<String>,

    // Privy — consumer authentication
    /// Privy App ID from https://dashboard.privy.io (your-app-id).
    /// When set, all `/app/*` consumer routes require a valid Privy session.
    /// Leave unset for single-operator deployments (no per-user auth).
    pub privy_app_id:             Option<String>,
    /// WalletConnect Cloud project ID — enables mobile wallet login (MetaMask Mobile,
    /// Rainbow, Coinbase Wallet, etc.) via Privy's wallet login method.
    /// Get a free project ID at https://cloud.walletconnect.com
    /// When unset, browser-extension wallets (MetaMask desktop) still work.
    pub walletconnect_project_id: Option<String>,
    /// HMAC-SHA256 key used to sign session cookies.
    /// Generate with: `openssl rand -hex 32`
    /// Falls back to a random UUID at startup if not set (sessions survive
    /// only until the next server restart in that case).
    pub session_secret:  String,

    // Apple Pay — domain verification
    /// Contents of the Apple Pay domain-association file from Stripe Dashboard.
    /// Served at `/.well-known/apple-developer-merchantid-domain-association`.
    /// Get it: Stripe Dashboard → Settings → Payment methods → Apple Pay →
    /// Add new domain → copy the file contents into this env var.
    pub apple_pay_domain_assoc: Option<String>,

    // Admin panel — HTTP Basic Auth
    /// Password that protects `/admin/*` routes.
    /// Accessed as `ADMIN_PASSWORD` env var.  The username is always "admin".
    /// When unset, the admin panel returns 503 (not configured).
    /// Generate a strong password: `openssl rand -hex 16`
    pub admin_password: Option<String>,
    /// Coinzilla publisher zone ID — shown only to Free/Trial users.
    /// Get from: publishers.coinzilla.io → My Sites → Zone → Get Code.
    /// The numeric ID in the script tag, e.g. "12345".
    /// When unset, no ads are rendered anywhere.
    pub coinzilla_zone_id: Option<String>,

    // Affiliate — Hyperliquid referral code
    /// Referral slug registered at app.hyperliquid.xyz (e.g. "TRADINGBOTS").
    /// Displayed in the consumer /app page so new users sign up via our link.
    /// Earns 10 % fee rebate on all trading volume from referred users.
    pub referral_code: Option<String>,

    // Database – optional
    pub database_url: String,

    // Risk
    pub max_concurrent_trades: usize,

    // Paper-trading flag
    pub paper_trading: bool,

    // Transactional email — Resend API (https://resend.com)
    /// Resend API key (`re_…`).  When unset, all email sending is silently skipped.
    /// Get one free at resend.com — 100 emails/day on the free plan.
    pub email_api_key: Option<String>,
    /// Sender address shown to users (RFC 5322 name+addr format).
    /// Defaults to "TradingBots.fun <hello@tradingbots.fun>".
    /// Set via `EMAIL_FROM` env var.
    pub email_from: Option<String>,

    // Stripe — introductory promo price for trial-expired users
    /// Stripe Price ID for the $9.95 first-month introductory offer.
    /// When set, `/billing/checkout?promo=1` uses this price instead of
    /// the standard `stripe_price_id` ($19.99/month).
    /// Set via `STRIPE_PROMO_PRICE_ID` env var.
    pub stripe_promo_price_id: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let mode_str = env::var("MODE").unwrap_or_else(|_| "paper".to_string());
        let mode = match mode_str.to_lowercase().as_str() {
            "mainnet" => Mode::Mainnet,
            "testnet" => Mode::Testnet,
            _         => Mode::Paper,
        };

        let paper_trading = matches!(mode, Mode::Paper)
            || env::var("PAPER_TRADING").unwrap_or_else(|_| "true".to_string()) == "true";

        let symbols_str = env::var("TRADING_SYMBOLS").unwrap_or_else(|_| "ALL".to_string());
        let trading_symbols = if symbols_str.to_uppercase() == "ALL" {
            vec!["ALL".to_string()]
        } else {
            symbols_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        Ok(Config {
            mode,
            trading_symbol: env::var("TRADING_SYMBOL").unwrap_or_else(|_| "SOL".to_string()),
            trading_symbols,
            initial_capital: env::var("INITIAL_CAPITAL")
                .unwrap_or_else(|_| "1000.0".to_string())
                .parse()
                .unwrap_or(1000.0),
            max_position_pct: 0.15,
            max_leverage: 10.0,
            daily_loss_limit: 50.0,
            min_health_factor: 2.0,
            binance_api_key:            env::var("BINANCE_API_KEY").ok(),
            hyperliquid_key:            env::var("HYPERLIQUID_KEY").ok(),
            hyperliquid_secret:         env::var("HYPERLIQUID_SECRET").ok(),
            hyperliquid_wallet_address: env::var("HYPERLIQUID_WALLET_ADDRESS").ok(),
            builder_code:               env::var("BUILDER_CODE").ok(),
            builder_fee_bps:            env::var("BUILDER_FEE_BPS")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(1)
                .min(3), // HL hard cap
            referral_code:              env::var("REFERRAL_CODE").ok(),
            stripe_secret_key:          env::var("STRIPE_SECRET_KEY").ok(),
            stripe_webhook_secret:      env::var("STRIPE_WEBHOOK_SECRET").ok(),
            stripe_price_id:            env::var("STRIPE_PRICE_ID").ok(),
            privy_app_id:               env::var("PRIVY_APP_ID").ok(),
            walletconnect_project_id:   env::var("WALLETCONNECT_PROJECT_ID").ok(),
            session_secret:             env::var("SESSION_SECRET")
                .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string()),
            apple_pay_domain_assoc:     env::var("APPLE_PAY_DOMAIN_ASSOC").ok(),
            admin_password:             env::var("ADMIN_PASSWORD").ok(),
            coinzilla_zone_id:          env::var("COINZILLA_ZONE_ID").ok(),
            email_api_key:              env::var("RESEND_API_KEY").ok(),
            email_from:                 env::var("EMAIL_FROM").ok(),
            stripe_promo_price_id:      env::var("STRIPE_PROMO_PRICE_ID").ok(),
            lunarcrush_api_key:         env::var("LUNARCRUSH_API_KEY")
                .unwrap_or_else(|_| "77c4fcm050bnxe49qo1h2n252umls0rrtkevh5uni".to_string()),
            anthropic_api_key:          env::var("ANTHROPIC_API_KEY").ok(),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:///var/data/tradingbots.db".to_string()),
            max_concurrent_trades: 3,
            paper_trading,
        })
    }
}
