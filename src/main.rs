//! RedRobot HedgeBot - Autonomous Cryptocurrency Trading System
//! Real-money trading on Solana DEX (Hyperliquid + Drift)

mod config;
mod data;
mod indicators;
mod signals;
mod risk;
mod exchange;
mod monitoring;
mod decision;
mod db;

use anyhow::Result;
use log::{info, error};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    info!("🤖 RedRobot HedgeBot Starting (Real Money Mode)");

    // Load config
    let config = config::Config::from_env()?;
    info!("✓ Configuration loaded: {:?}", config.mode);

    // Initialize database
    let db = Arc::new(db::Database::new(&config).await?);
    info!("✓ Database initialized");

    // Initialize CEX client
    let cex_client = Arc::new(data::CexClient::new(&config)?);
    info!("✓ CEX client initialized");

    // Initialize Hyperliquid client
    let hl_client = Arc::new(exchange::HyperliquidClient::new(&config)?);
    info!("✓ Hyperliquid client initialized");

    // Shutdown signal
    let running = Arc::new(RwLock::new(true));
    let running_clone = running.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received");
        *running_clone.write().await = false;
    });

    // Main trading loop
    loop {
        if !*running.read().await {
            break;
        }

        match run_cycle(&config, &cex_client, &hl_client, &db).await {
            Ok(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(e) => {
                error!("Cycle error: {}", e);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    info!("🛑 RedRobot shutdown complete");
    Ok(())
}

async fn run_cycle(
    config: &config::Config,
    cex_client: &std::sync::Arc<data::CexClient>,
    hl_client: &std::sync::Arc<exchange::HyperliquidClient>,
    db: &std::sync::Arc<db::Database>,
) -> Result<()> {
    // Fetch market data
    let market_data = cex_client.fetch_market_data(&config.trading_symbol).await?;

    // Calculate indicators
    let indicators = indicators::calculate_all(&market_data)?;

    // Detect signals
    let order_book = cex_client.fetch_order_book(&config.trading_symbol).await?;
    let order_flow = signals::detect_order_flow(&order_book)?;

    // Make decision
    let decision = decision::make_decision(&market_data, &indicators, &order_flow)?;

    // Risk management
    let account = hl_client.get_account().await?;
    if risk::should_trade(&decision, &account)? {
        match hl_client.place_order(&decision).await {
            Ok(order_id) => {
                info!("✓ Order placed: {}", order_id);
                db.log_trade(&decision, &order_id).await.ok();
            }
            Err(e) => {
                error!("Order failed: {}", e);
            }
        }
    }

    // Monitor positions
    let positions = hl_client.get_positions().await?;
    for pos in positions {
        if pos.should_close() {
            hl_client.close_position(&pos).await.ok();
        }
    }

    Ok(())
}
