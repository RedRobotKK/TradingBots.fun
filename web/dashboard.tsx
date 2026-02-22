/**
 * 🎨 RedRobot Trading Bot - Web Dashboard
 * Responsive design: Works perfect on desktop AND mobile
 * Real-time updates via WebSocket
 *
 * Features:
 * - Live equity tracking with charts
 * - Current position monitoring
 * - Recent trade history with detailed P&L
 * - AI decision reasoning and confidence
 * - Sentiment analysis (Fear/Greed)
 * - Market regime detection
 * - System alerts and error logs
 * - Mobile-optimized layout
 */

import React, { useState, useEffect, useRef } from 'react';
import './dashboard.css';

// ============================================================================
// TYPE DEFINITIONS
// ============================================================================

interface DashboardMetrics {
  initial_equity: number;
  current_equity: number;
  daily_pnl: number;
  total_pnl: number;
  return_pct: number;
  max_drawdown_pct: number;
  total_trades: number;
  winning_trades: number;
  losing_trades: number;
  win_rate: number;
  average_win: number;
  average_loss: number;
  profit_factor: number;
  has_position: boolean;
  position_symbol?: string;
  position_entry_price?: number;
  position_current_price?: number;
  position_size?: number;
  position_pnl?: number;
  position_pnl_pct?: number;
  position_time_open?: string;
  current_price: number;
  support_level: number;
  resistance_level: number;
  atr_pct: number;
  bid_ask_ratio: number;
  fear_greed_index: number;
  rsi: number;
  macd_signal: boolean;
  confluence_score: number;
}

interface RecentTrade {
  timestamp: string;
  symbol: string;
  action: string;
  entry_price: number;
  exit_price?: number;
  size: number;
  pnl: number;
  pnl_pct: number;
  strategy: string;
  confidence: number;
  status: string;
}

interface AIThoughts {
  current_signal: string;
  technical_confidence: number;
  framework_validation: Array<[string, boolean]>;
  reasoning: string[];
  warnings: string[];
  next_check_seconds: number;
}

interface SystemAlert {
  level: 'Info' | 'Warning' | 'Error' | 'Critical';
  timestamp: string;
  message: string;
}

// ============================================================================
// MAIN DASHBOARD COMPONENT
// ============================================================================

export const TradingDashboard: React.FC = () => {
  const [metrics, setMetrics] = useState<DashboardMetrics | null>(null);
  const [recentTrades, setRecentTrades] = useState<RecentTrade[]>([]);
  const [aiThoughts, setAiThoughts] = useState<AIThoughts | null>(null);
  const [alerts, setAlerts] = useState<SystemAlert[]>([]);
  const [equityHistory, setEquityHistory] = useState<number[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);

  // Connect to WebSocket on mount
  useEffect(() => {
    const connectWebSocket = () => {
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const wsUrl = `${protocol}//${window.location.host}/ws/dashboard`;

      wsRef.current = new WebSocket(wsUrl);

      wsRef.current.onopen = () => {
        console.log('Connected to trading bot');
        setIsConnected(true);
      };

      wsRef.current.onmessage = (event) => {
        const data = JSON.parse(event.data);

        if (data.metrics) setMetrics(data.metrics);
        if (data.recent_trades) setRecentTrades(data.recent_trades);
        if (data.ai_thoughts) setAiThoughts(data.ai_thoughts);
        if (data.alerts) setAlerts(data.alerts);

        // Update equity history for chart
        if (data.metrics?.current_equity) {
          setEquityHistory((prev) => [...prev.slice(-49), data.metrics.current_equity]);
        }
      };

      wsRef.current.onerror = () => setIsConnected(false);
      wsRef.current.onclose = () => {
        setIsConnected(false);
        setTimeout(connectWebSocket, 3000); // Reconnect after 3s
      };
    };

    connectWebSocket();

    return () => {
      if (wsRef.current) wsRef.current.close();
    };
  }, []);

  if (!metrics) {
    return (
      <div className="dashboard-loading">
        <div className="spinner"></div>
        <h2>Connecting to trading bot...</h2>
        <p className="status-disconnected">Status: {isConnected ? 'Connected' : 'Disconnected'}</p>
      </div>
    );
  }

  return (
    <div className="dashboard-container">
      {/* Header */}
      <header className="dashboard-header">
        <div className="header-left">
          <h1>🤖 RedRobot Trading Bot</h1>
          <div className={`connection-status ${isConnected ? 'connected' : 'disconnected'}`}>
            <span className="status-dot"></span>
            {isConnected ? 'Live' : 'Offline'}
          </div>
        </div>
        <div className="header-right">
          <div className="equity-display">
            <div className="equity-amount">${metrics.current_equity.toFixed(2)}</div>
            <div className={`equity-change ${metrics.total_pnl >= 0 ? 'positive' : 'negative'}`}>
              {metrics.total_pnl >= 0 ? '+' : ''}{metrics.total_pnl.toFixed(2)} ({metrics.return_pct.toFixed(2)}%)
            </div>
          </div>
        </div>
      </header>

      {/* Main Grid */}
      <main className="dashboard-grid">
        {/* Row 1: Key Metrics */}
        <section className="metrics-row">
          <MetricCard label="Total Trades" value={metrics.total_trades.toString()} />
          <MetricCard label="Win Rate" value={`${(metrics.win_rate * 100).toFixed(1)}%`} />
          <MetricCard label="Profit Factor" value={metrics.profit_factor.toFixed(2)} />
          <MetricCard label="Max Drawdown" value={`${metrics.max_drawdown_pct.toFixed(1)}%`} />
          <MetricCard label="Daily P&L" value={`$${metrics.daily_pnl.toFixed(2)}`} />
        </section>

        {/* Row 2: Current Position */}
        {metrics.has_position && (
          <section className="position-card">
            <h2>📍 Current Position</h2>
            <PositionDetail metrics={metrics} />
          </section>
        )}

        {/* Row 3: Market State (2 columns) */}
        <section className="market-grid">
          <div className="card market-card">
            <h2>📈 Market State</h2>
            <MarketState metrics={metrics} />
          </div>

          <div className="card sentiment-card">
            <h2>😨 Sentiment & Fear/Greed</h2>
            <SentimentDisplay metrics={metrics} />
          </div>
        </section>

        {/* Row 4: AI Decision Engine */}
        <section className="card ai-card">
          <h2>🧠 What AI is Thinking</h2>
          {aiThoughts && <AIThinking thoughts={aiThoughts} />}
        </section>

        {/* Row 5: Equity Chart */}
        <section className="card chart-card">
          <h2>📊 Equity Curve (Last 50 Updates)</h2>
          <EquityChart history={equityHistory} />
        </section>

        {/* Row 6: Recent Trades */}
        <section className="card trades-card">
          <h2>📋 Recent Trades</h2>
          <TradeHistory trades={recentTrades} />
        </section>

        {/* Row 7: Alerts & System Status */}
        <section className="card alerts-card">
          <h2>⚠️ Alerts & System Status</h2>
          <AlertList alerts={alerts} />
        </section>
      </main>
    </div>
  );
};

// ============================================================================
// SUB-COMPONENTS
// ============================================================================

const MetricCard: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <div className="metric-card">
    <div className="metric-label">{label}</div>
    <div className="metric-value">{value}</div>
  </div>
);

const PositionDetail: React.FC<{ metrics: DashboardMetrics }> = ({ metrics }) => (
  <div className="position-details">
    <div className="position-row">
      <div className="position-item">
        <span className="label">Symbol</span>
        <span className="value">{metrics.position_symbol}</span>
      </div>
      <div className="position-item">
        <span className="label">Entry</span>
        <span className="value">${metrics.position_entry_price?.toFixed(2)}</span>
      </div>
      <div className="position-item">
        <span className="label">Current</span>
        <span className="value">${metrics.position_current_price?.toFixed(2)}</span>
      </div>
      <div className="position-item">
        <span className="label">Size</span>
        <span className="value">{metrics.position_size?.toFixed(4)} {metrics.position_symbol}</span>
      </div>
    </div>
    <div className="position-row">
      <div className="position-item">
        <span className="label">Stop Loss</span>
        <span className="value error">No SL set</span>
      </div>
      <div className="position-item">
        <span className="label">Take Profit</span>
        <span className="value">No TP set</span>
      </div>
      <div className={`position-item pnl-item ${(metrics.position_pnl || 0) >= 0 ? 'positive' : 'negative'}`}>
        <span className="label">P&L</span>
        <span className="value">${metrics.position_pnl?.toFixed(2)} ({metrics.position_pnl_pct?.toFixed(2)}%)</span>
      </div>
      <div className="position-item">
        <span className="label">Leverage</span>
        <span className="value">10.0x</span>
      </div>
    </div>
  </div>
);

const MarketState: React.FC<{ metrics: DashboardMetrics }> = ({ metrics }) => (
  <div className="market-state">
    <div className="state-row">
      <span className="state-label">Price</span>
      <span className="state-value">${metrics.current_price.toFixed(2)}</span>
    </div>
    <div className="state-row">
      <span className="state-label">Support / Resistance</span>
      <span className="state-value">
        ${metrics.support_level.toFixed(2)} / ${metrics.resistance_level.toFixed(2)}
      </span>
    </div>
    <div className="state-row">
      <span className="state-label">ATR</span>
      <span className="state-value">{metrics.atr_pct.toFixed(2)}%</span>
    </div>
    <div className="state-row">
      <span className="state-label">Bid/Ask Ratio</span>
      <span className="state-value">{metrics.bid_ask_ratio.toFixed(2)}x</span>
    </div>
    <div className="state-row">
      <span className="state-label">Confluence Score</span>
      <span className="state-value">{(metrics.confluence_score * 100).toFixed(0)}%</span>
    </div>
  </div>
);

const SentimentDisplay: React.FC<{ metrics: DashboardMetrics }> = ({ metrics }) => {
  const getSentimentColor = (index: number) => {
    if (index < 25) return '#d32f2f'; // Extreme fear - red
    if (index < 45) return '#f57c00'; // Fear - orange
    if (index < 55) return '#fbc02d'; // Neutral - yellow
    if (index < 75) return '#7cb342'; // Greed - light green
    return '#388e3c'; // Extreme greed - green
  };

  const getSentimentLabel = (index: number) => {
    if (index < 25) return 'EXTREME FEAR';
    if (index < 45) return 'FEAR';
    if (index < 55) return 'NEUTRAL';
    if (index < 75) return 'GREED';
    return 'EXTREME GREED';
  };

  return (
    <div className="sentiment-display">
      <div className="fear-greed">
        <div className="fear-greed-bar">
          <div
            className="fear-greed-indicator"
            style={{
              left: `${metrics.fear_greed_index}%`,
              backgroundColor: getSentimentColor(metrics.fear_greed_index),
            }}
          ></div>
        </div>
        <div className="fear-greed-labels">
          <span>Fear</span>
          <span className="fear-greed-value">{metrics.fear_greed_index}</span>
          <span>Greed</span>
        </div>
        <div className="fear-greed-label">{getSentimentLabel(metrics.fear_greed_index)}</div>
      </div>

      <div className="sentiment-indicators">
        <div className="indicator-row">
          <span>RSI</span>
          <span className="indicator-value">{metrics.rsi.toFixed(0)}</span>
          <span className="indicator-status">{metrics.rsi < 30 ? '📉 Oversold' : metrics.rsi > 70 ? '📈 Overbought' : '⚪ Neutral'}</span>
        </div>
        <div className="indicator-row">
          <span>MACD</span>
          <span className="indicator-value">{metrics.macd_signal ? '📈 Bullish' : '📉 Bearish'}</span>
        </div>
      </div>
    </div>
  );
};

const AIThinking: React.FC<{ thoughts: AIThoughts }> = ({ thoughts }) => (
  <div className="ai-thinking">
    <div className="ai-header">
      <h3 className="ai-signal">{thoughts.current_signal}</h3>
      <div className="ai-confidence">
        Confidence: <strong>{(thoughts.technical_confidence * 100).toFixed(0)}%</strong>
      </div>
    </div>

    <div className="frameworks">
      <h4>Framework Validation</h4>
      <div className="framework-list">
        {thoughts.framework_validation.map(([name, passed]) => (
          <div key={name} className={`framework-item ${passed ? 'passed' : 'failed'}`}>
            <span className="framework-icon">{passed ? '✓' : '✗'}</span>
            <span>{name}</span>
          </div>
        ))}
      </div>
    </div>

    <div className="reasoning">
      <h4>Reasoning</h4>
      <ul>
        {thoughts.reasoning.map((reason, idx) => (
          <li key={idx}>• {reason}</li>
        ))}
      </ul>
    </div>

    {thoughts.warnings.length > 0 && (
      <div className="warnings">
        <h4>⚠️ Warnings</h4>
        <ul>
          {thoughts.warnings.map((warning, idx) => (
            <li key={idx}>⚠️ {warning}</li>
          ))}
        </ul>
      </div>
    )}

    <div className="next-check">Next decision in {thoughts.next_check_seconds}s</div>
  </div>
);

const EquityChart: React.FC<{ history: number[] }> = ({ history }) => {
  if (history.length === 0) return <p>Waiting for data...</p>;

  const min = Math.min(...history);
  const max = Math.max(...history);
  const range = max - min || 1;

  return (
    <div className="equity-chart">
      <svg viewBox={`0 0 ${history.length * 5} 100`} preserveAspectRatio="xMidYMid slice">
        <polyline
          points={history
            .map((value, idx) => {
              const y = 100 - ((value - min) / range) * 90 - 5;
              return `${idx * 5},${y}`;
            })
            .join(' ')}
          fill="none"
          stroke="#4CAF50"
          strokeWidth="2"
        />
      </svg>
    </div>
  );
};

const TradeHistory: React.FC<{ trades: RecentTrade[] }> = ({ trades }) => (
  <div className="trade-history">
    <div className="trade-table-wrapper">
      <table className="trade-table">
        <thead>
          <tr>
            <th>Time</th>
            <th>Action</th>
            <th>Price</th>
            <th>Size</th>
            <th>P&L</th>
            <th>Strategy</th>
            <th>Status</th>
          </tr>
        </thead>
        <tbody>
          {trades.slice(-10).reverse().map((trade, idx) => (
            <tr key={idx} className={`trade-row ${trade.pnl >= 0 ? 'win' : 'loss'}`}>
              <td className="time">{new Date(trade.timestamp).toLocaleTimeString()}</td>
              <td className="action">{trade.action}</td>
              <td className="price">${trade.entry_price.toFixed(2)}</td>
              <td className="size">{trade.size.toFixed(4)}</td>
              <td className={`pnl ${trade.pnl >= 0 ? 'positive' : 'negative'}`}>
                {trade.pnl >= 0 ? '+' : ''} ${trade.pnl.toFixed(2)} ({trade.pnl_pct.toFixed(2)}%)
              </td>
              <td className="strategy">{trade.strategy}</td>
              <td className="status">{trade.status}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  </div>
);

const AlertList: React.FC<{ alerts: SystemAlert[] }> = ({ alerts }) => (
  <div className="alert-list">
    {alerts.slice(-5).reverse().map((alert, idx) => (
      <div key={idx} className={`alert alert-${alert.level.toLowerCase()}`}>
        <span className="alert-icon">
          {alert.level === 'Info' && 'ℹ️'}
          {alert.level === 'Warning' && '⚠️'}
          {alert.level === 'Error' && '❌'}
          {alert.level === 'Critical' && '🚨'}
        </span>
        <div className="alert-content">
          <div className="alert-time">{new Date(alert.timestamp).toLocaleTimeString()}</div>
          <div className="alert-message">{alert.message}</div>
        </div>
      </div>
    ))}
  </div>
);
