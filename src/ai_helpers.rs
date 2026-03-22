use crate::learner::SignalContribution;
use crate::web_dashboard::PaperPosition;

/// Returns `↑` when the signal is bullish and `↓` when bearish.
pub(crate) fn signal_direction(flag: bool) -> &'static str {
    if flag {
        "↑"
    } else {
        "↓"
    }
}

/// Human-readable summary of all signal votes.
pub fn signal_breakdown(contrib: &SignalContribution) -> String {
    let mut pieces = Vec::new();
    pieces.push(format!("RSI{}", signal_direction(contrib.rsi_bullish)));
    pieces.push(format!("BB{}", signal_direction(contrib.bb_bullish)));
    pieces.push(format!("MACD{}", signal_direction(contrib.macd_bullish)));
    pieces.push(format!(
        "EMA{}",
        signal_direction(contrib.ema_cross_bullish)
    ));
    pieces.push(format!("Trend{}", signal_direction(contrib.trend_bullish)));
    pieces.push(format!("OF{}", signal_direction(contrib.of_bullish)));

    if contrib.z_score_present {
        pieces.push(format!("Z{}", signal_direction(contrib.z_score_bullish)));
    }
    if contrib.volume_present {
        pieces.push(format!("Vol{}", signal_direction(contrib.volume_bullish)));
    }
    if contrib.sentiment_present {
        pieces.push(format!(
            "Sent{}",
            signal_direction(contrib.sentiment_bullish)
        ));
    }
    if contrib.funding_present {
        pieces.push(format!("Fund{}", signal_direction(contrib.funding_bullish)));
    }

    pieces.join(" | ")
}

/// Difference between bullish and bearish votes, normalised to [−1.0, +1.0].
pub fn signal_alignment_pct(contrib: &SignalContribution) -> f64 {
    let mut total = 0;
    let mut bullish = 0;

    let mut tally = |present: bool, bullish_flag: bool| {
        if present {
            total += 1;
            if bullish_flag {
                bullish += 1;
            }
        }
    };

    tally(true, contrib.rsi_bullish);
    tally(true, contrib.bb_bullish);
    tally(true, contrib.macd_bullish);
    tally(true, contrib.ema_cross_bullish);
    tally(true, contrib.trend_bullish);
    tally(true, contrib.of_bullish);
    tally(contrib.z_score_present, contrib.z_score_bullish);
    tally(contrib.volume_present, contrib.volume_bullish);
    tally(contrib.sentiment_present, contrib.sentiment_bullish);
    tally(contrib.funding_present, contrib.funding_bullish);

    if total == 0 {
        0.0
    } else {
        ((bullish as f64) - ((total - bullish) as f64)) / total as f64
    }
}

/// Summary of the current order-flow snapshot captured on the position.
pub fn order_flow_snapshot(pos: &PaperPosition) -> String {
    let direction = if pos.order_flow_direction.is_empty() {
        "NEUTRAL"
    } else {
        pos.order_flow_direction.as_str()
    };
    let confidence = (pos.order_flow_confidence * 100.0).round();
    let mut walls = Vec::new();
    if pos.ob_bid_wall_near {
        walls.push("bid-wall");
    } else {
        walls.push("no bid-wall");
    }
    if pos.ob_ask_wall_near {
        walls.push("ask-wall");
    } else {
        walls.push("no ask-wall");
    }
    format!(
        "{} ({:.0}% conf) | {} | Sentiment: {} | Adverse cycles: {}",
        direction,
        confidence,
        walls.join(" & "),
        if pos.ob_sentiment.is_empty() {
            "NEUTRAL"
        } else {
            pos.ob_sentiment.as_str()
        },
        pos.ob_adverse_cycles
    )
}

/// Cross-exchange divergence summary for the position.
pub fn cross_exchange_snapshot(pos: &PaperPosition) -> String {
    if pos.cex_mode.is_empty() {
        "Inactive".to_string()
    } else {
        format!("{} ({:+.2}% premium)", pos.cex_mode, pos.cex_premium_pct)
    }
}

/// Describes the funding rate, cycle label, and time until settlement.
#[allow(dead_code)]
pub fn funding_context(pos: &PaperPosition, phase_label: &str, hours_to_settlement: f64) -> String {
    format!(
        "{:+.2}% (Δ{:+.2}%) · {} · {:.1}h",
        pos.funding_rate * 100.0,
        pos.funding_delta * 100.0,
        phase_label,
        hours_to_settlement,
    )
}
