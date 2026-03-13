/// Autonomous trading bot runner
use crate::modules::{AccountManager, CapitalManager, LiquidationPrevention};
use crate::utils::{Error, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, warn};
use rand;

/// Autonomous trading system state
pub struct AutonomousRunner {
    account_manager: Arc<AccountManager>,
    capital_manager: Arc<CapitalManager>,
    liquidation_prevention: Arc<LiquidationPrevention>,
    state: Arc<RwLock<RunnerState>>,
    decision_interval_ms: u64,
    rebalance_interval_ms: u64,
    health_check_interval_ms: u64,
}

#[derive(Clone, Debug)]
pub struct RunnerState {
    pub is_running: bool,
    pub status: SystemStatus,
    pub last_decision: Option<i64>,
    pub last_rebalance: Option<i64>,
    pub total_decisions: u64,
    pub winning_decisions: u64,
    pub cumulative_pnl: f64,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemStatus {
    Initializing,
    Running,
    HealthCheckFailed,
    HighRisk,
    Paused,
    EmergencyStop,
    Stopped,
}

impl Default for RunnerState {
    fn default() -> Self {
        Self {
            is_running: false,
            status: SystemStatus::Initializing,
            last_decision: None,
            last_rebalance: None,
            total_decisions: 0,
            winning_decisions: 0,
            cumulative_pnl: 0.0,
            last_error: None,
        }
    }
}

impl AutonomousRunner {
    /// Create new autonomous runner
    pub fn new(
        account_manager: Arc<AccountManager>,
        capital_manager: Arc<CapitalManager>,
        liquidation_prevention: Arc<LiquidationPrevention>,
    ) -> Self {
        Self {
            account_manager,
            capital_manager,
            liquidation_prevention,
            state: Arc::new(RwLock::new(RunnerState::default())),
            decision_interval_ms: 1000,
            rebalance_interval_ms: 300000, // 5 minutes
            health_check_interval_ms: 5000, // 5 seconds
        }
    }

    /// Start the autonomous runner
    pub async fn start(&self) -> Result<()> {
        let mut state = self.state.write().await;

        if state.is_running {
            return Err(Error::InternalError("Already running".to_string()));
        }

        state.is_running = true;
        state.status = SystemStatus::Running;

        info!("🤖 Autonomous trader started");

        // Spawn background tasks
        let state_clone = self.state.clone();
        let liq_prev = self.liquidation_prevention.clone();

        tokio::spawn(async move {
            Self::health_check_loop(state_clone, liq_prev).await;
        });

        let state_clone = self.state.clone();
        let capital_mgr = self.capital_manager.clone();

        tokio::spawn(async move {
            Self::rebalance_loop(state_clone, capital_mgr).await;
        });

        let state_clone = self.state.clone();

        tokio::spawn(async move {
            Self::decision_loop(state_clone).await;
        });

        Ok(())
    }

    /// Stop the autonomous runner
    pub async fn stop(&self) -> Result<()> {
        let mut state = self.state.write().await;
        state.is_running = false;
        state.status = SystemStatus::Stopped;

        info!("🛑 Autonomous trader stopped");

        Ok(())
    }

    /// Pause trading (keep monitoring)
    pub async fn pause(&self) -> Result<()> {
        let mut state = self.state.write().await;
        state.status = SystemStatus::Paused;

        warn!("⏸  Trading paused (monitoring continues)");

        Ok(())
    }

    /// Resume trading
    pub async fn resume(&self) -> Result<()> {
        let mut state = self.state.write().await;

        if state.status != SystemStatus::Paused {
            return Err(Error::InternalError("Not paused".to_string()));
        }

        state.status = SystemStatus::Running;

        info!("▶️  Trading resumed");

        Ok(())
    }

    /// Emergency stop (liquidation protection)
    pub async fn emergency_stop(&self) -> Result<()> {
        let mut state = self.state.write().await;
        state.status = SystemStatus::EmergencyStop;
        state.last_error = Some("Emergency stop triggered".to_string());

        error!("🚨 EMERGENCY STOP - All trading halted");

        Ok(())
    }

    /// Get current state
    pub async fn get_state(&self) -> RunnerState {
        self.state.read().await.clone()
    }

    /// Decision loop - makes trading decisions
    async fn decision_loop(state: Arc<RwLock<RunnerState>>) {
        loop {
            sleep(Duration::from_millis(1000)).await;

            let mut s = state.write().await;

            if !s.is_running {
                break;
            }

            // In real system: Make AI decision here
            // For now: Simulate decision making
            s.last_decision = Some(chrono::Utc::now().timestamp());
            s.total_decisions += 1;

            // Simulate win rate
            if rand::random::<f64>() > 0.4 {
                s.winning_decisions += 1;
                s.cumulative_pnl += rand::random::<f64>() * 100.0;
            } else {
                s.cumulative_pnl -= rand::random::<f64>() * 50.0;
            }

            if s.total_decisions % 60 == 0 {
                let win_rate = (s.winning_decisions as f64 / s.total_decisions as f64) * 100.0;
                info!(
                    "📊 Decisions: {}, Win Rate: {:.1}%, P&L: ${:.2}",
                    s.total_decisions, win_rate, s.cumulative_pnl
                );
            }
        }
    }

    /// Health check loop
    async fn health_check_loop(
        state: Arc<RwLock<RunnerState>>,
        liq_prev: Arc<LiquidationPrevention>,
    ) {
        loop {
            sleep(Duration::from_millis(5000)).await;

            let s = state.read().await;

            if !s.is_running {
                break;
            }

            // Simulate account health check
            let health_factor = 1.5 + (rand::random::<f64>() - 0.5) * 0.5; // Range: 1.25-1.75

            if liq_prev.needs_action(health_factor) {
                let alert = liq_prev
                    .monitor_account("primary", health_factor)
                    .await
                    .ok()
                    .flatten();

                if let Some(alert) = alert {
                    warn!(
                        "⚠️  Risk Alert: {} (Health: {:.2})",
                        alert.reason, alert.health_factor
                    );
                }
            }
        }
    }

    /// Rebalance loop
    async fn rebalance_loop(
        state: Arc<RwLock<RunnerState>>,
        capital_mgr: Arc<CapitalManager>,
    ) {
        loop {
            sleep(Duration::from_millis(300000)).await; // 5 minutes

            let mut s = state.write().await;

            if !s.is_running || s.status == SystemStatus::Paused {
                continue;
            }

            // Rebalance capital
            let volatility = 0.1 + (rand::random::<f64>() - 0.5) * 0.1;
            let momentum = (rand::random::<f64>() - 0.5) * 0.3;
            let win_rate = s.winning_decisions as f64 / s.total_decisions.max(1) as f64;

            if let Ok(_new_allocs) = capital_mgr
                .optimize_allocation(volatility, momentum, win_rate)
                .await
            {
                s.last_rebalance = Some(chrono::Utc::now().timestamp());

                info!(
                    "♻️  Rebalanced: Volatility={:.2}%, Momentum={:.2}, Win Rate={:.1}%",
                    volatility * 100.0,
                    momentum,
                    win_rate * 100.0
                );
            }
        }
    }

    /// Performance metrics
    pub async fn get_performance(&self) -> PerformanceMetrics {
        let state = self.state.read().await;

        let win_rate = if state.total_decisions > 0 {
            (state.winning_decisions as f64 / state.total_decisions as f64) * 100.0
        } else {
            0.0
        };

        PerformanceMetrics {
            total_decisions: state.total_decisions,
            winning_decisions: state.winning_decisions,
            win_rate,
            cumulative_pnl: state.cumulative_pnl,
            status: state.status,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PerformanceMetrics {
    pub total_decisions: u64,
    pub winning_decisions: u64,
    pub win_rate: f64,
    pub cumulative_pnl: f64,
    pub status: SystemStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_runner_creation() {
        let acc_mgr = Arc::new(AccountManager::new());
        let cap_mgr = Arc::new(CapitalManager::new(5000.0));
        let liq_prev = Arc::new(LiquidationPrevention::default());

        let runner = AutonomousRunner::new(acc_mgr, cap_mgr, liq_prev);
        let state = runner.get_state().await;

        assert!(!state.is_running);
    }

    #[tokio::test]
    async fn test_start_stop() {
        let acc_mgr = Arc::new(AccountManager::new());
        let cap_mgr = Arc::new(CapitalManager::new(5000.0));
        let liq_prev = Arc::new(LiquidationPrevention::default());

        let runner = AutonomousRunner::new(acc_mgr, cap_mgr, liq_prev);

        runner.start().await.unwrap();
        let state = runner.get_state().await;
        assert!(state.is_running);

        runner.stop().await.unwrap();
        let state = runner.get_state().await;
        assert!(!state.is_running);
    }

    #[tokio::test]
    async fn test_pause_resume() {
        let acc_mgr = Arc::new(AccountManager::new());
        let cap_mgr = Arc::new(CapitalManager::new(5000.0));
        let liq_prev = Arc::new(LiquidationPrevention::default());

        let runner = AutonomousRunner::new(acc_mgr, cap_mgr, liq_prev);
        runner.start().await.unwrap();

        runner.pause().await.unwrap();
        let state = runner.get_state().await;
        assert_eq!(state.status, SystemStatus::Paused);

        runner.resume().await.unwrap();
        let state = runner.get_state().await;
        assert_eq!(state.status, SystemStatus::Running);
    }

    #[tokio::test]
    async fn test_emergency_stop() {
        let acc_mgr = Arc::new(AccountManager::new());
        let cap_mgr = Arc::new(CapitalManager::new(5000.0));
        let liq_prev = Arc::new(LiquidationPrevention::default());

        let runner = AutonomousRunner::new(acc_mgr, cap_mgr, liq_prev);
        runner.start().await.unwrap();

        runner.emergency_stop().await.unwrap();
        let state = runner.get_state().await;
        assert_eq!(state.status, SystemStatus::EmergencyStop);
    }
}
