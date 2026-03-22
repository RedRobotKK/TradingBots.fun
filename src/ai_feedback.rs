use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedAiFeedbackLogger = Arc<Mutex<AiFeedbackLogger>>;

/// Guardrail evaluation history for automated learning / analytics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailFeedback {
    pub ts: String,
    pub symbol: String,
    pub action: String,
    pub recommendation: String,
    pub guardrail_score: f64,
    pub guardrail_components: Vec<String>,
    pub guardrail_note: Option<String>,
    pub guardrail_allowed: bool,
    pub r_multiple: f64,
    pub hold_minutes: u64,
    pub dca_remaining: u8,
    pub false_breakout: bool,
    pub momentum_stall: bool,
    pub entry_confidence: f64,
    pub signal_summary: String,
    pub signal_breakdown: String,
    pub signal_alignment_pct: f64,
    pub funding_phase: String,
    pub hours_to_settlement: f64,
    pub order_flow_snapshot: String,
    pub order_flow_confidence: f64,
    pub order_flow_direction: String,
    pub ob_sentiment: String,
    pub ob_adverse_cycles: u32,
    pub funding_rate: f64,
    pub funding_delta: f64,
    pub onchain_strength: f64,
    pub cex_premium_pct: f64,
    pub cex_mode: String,
    pub cross_exchange_snapshot: String,
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
}

/// Lightweight JSONL logger for AI guardrail feedback.
pub struct AiFeedbackLogger {
    file: File,
    _path: PathBuf,
}

impl AiFeedbackLogger {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { file, _path: path })
    }

    pub fn shared(path: impl AsRef<Path>) -> std::io::Result<SharedAiFeedbackLogger> {
        Ok(Arc::new(Mutex::new(Self::new(path)?)))
    }

    pub fn record(&mut self, entry: GuardrailFeedback) -> std::io::Result<()> {
        let line = serde_json::to_string(&entry)?;
        writeln!(self.file, "{}", line)?;
        self.file.flush()?;
        Ok(())
    }
}
