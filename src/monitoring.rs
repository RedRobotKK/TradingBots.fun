use log::{info, warn, error};

pub struct Monitor;

impl Monitor {
    pub fn log_decision(
        symbol: &str,
        action: &str,
        confidence: f64,
        strategy: &str,
    ) {
        let status = match action {
            "BUY" => "📈",
            "SELL" => "📉",
            _ => "⏸",
        };

        info!(
            "{} {} | Action: {} | Confidence: {:.2}% | Strategy: {}",
            status,
            symbol,
            action,
            confidence * 100.0,
            strategy
        );
    }

    pub fn log_error(error: &str) {
        error!("❌ {}", error);
    }

    pub fn log_warning(warning: &str) {
        warn!("⚠️  {}", warning);
    }

    pub fn log_info(info_msg: &str) {
        info!("ℹ️  {}", info_msg);
    }
}
