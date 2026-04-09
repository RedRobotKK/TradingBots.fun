/**
 * 🎯 Strategy Performance Dashboard Component
 * Real-time visualization of which strategies are winning
 * Mobile-responsive strategy attribution display
 */

import React, { useMemo } from 'react';
import './dashboard.css';

interface StrategyPerformance {
  strategy_name: string;
  viability_score: number;      // 0-100
  win_rate: number;              // 0-1
  profit_factor: number;
  total_signals: number;
  status: string;                // "Excellent" / "Good" / "Fair" / "Poor" / "Monitor"
  action: string;                // "Increase" / "Use" / "Monitor" / "Reduce" / "Remove"
}

interface StrategyPerformanceDashboardProps {
  strategies: StrategyPerformance[];
}

const StrategyPerformanceDashboard: React.FC<StrategyPerformanceDashboardProps> = ({ strategies }) => {
  // Sort strategies by viability score
  const sortedStrategies = useMemo(() => {
    return [...strategies].sort((a, b) => b.viability_score - a.viability_score);
  }, [strategies]);

  // Calculate summary statistics
  const summary = useMemo(() => {
    const total = strategies.length;
    const excellent = strategies.filter(s => s.viability_score >= 85).length;
    const good = strategies.filter(s => s.viability_score >= 70 && s.viability_score < 85).length;
    const fair = strategies.filter(s => s.viability_score >= 50 && s.viability_score < 70).length;
    const poor = strategies.filter(s => s.viability_score < 50).length;
    const avgScore = strategies.length > 0
      ? strategies.reduce((sum, s) => sum + s.viability_score, 0) / total
      : 0;

    return { total, excellent, good, fair, poor, avgScore };
  }, [strategies]);

  // Get color for viability score
  const getStatusColor = (score: number): string => {
    if (score >= 85) return '#00d084';  // Green - Excellent
    if (score >= 70) return '#ffb700';  // Yellow - Good
    if (score >= 50) return '#ff6b35';  // Orange - Fair
    return '#d62828';                   // Red - Poor
  };

  // Get status icon
  const getStatusIcon = (score: number): string => {
    if (score >= 85) return '🟢';
    if (score >= 70) return '🟡';
    if (score >= 50) return '🟠';
    return '🔴';
  };

  // Get action color
  const getActionColor = (action: string): string => {
    switch (action) {
      case 'Increase':
        return '#00d084';
      case 'Use':
        return '#00b4d8';
      case 'Monitor':
        return '#ffb700';
      case 'Reduce':
        return '#ff6b35';
      case 'Remove':
        return '#d62828';
      default:
        return '#999';
    }
  };

  return (
    <div className="strategy-performance-container">
      {/* Header */}
      <div className="card">
        <h2>🎯 Strategy Performance Attribution</h2>
        <p className="subtitle">Real-time tracking of which strategies are making money</p>

        {/* Summary Stats */}
        <div className="summary-grid">
          <div className="summary-stat">
            <div className="stat-label">Total Strategies</div>
            <div className="stat-value">{summary.total}</div>
          </div>
          <div className="summary-stat">
            <div className="stat-label">Avg Viability</div>
            <div className="stat-value" style={{ color: getStatusColor(summary.avgScore) }}>
              {summary.avgScore.toFixed(0)}
            </div>
          </div>
          <div className="summary-stat">
            <div className="stat-label">Excellent (85+)</div>
            <div className="stat-value" style={{ color: '#00d084' }}>
              {summary.excellent}
            </div>
          </div>
          <div className="summary-stat">
            <div className="stat-label">Needs Review</div>
            <div className="stat-value" style={{ color: '#ff6b35' }}>
              {summary.poor}
            </div>
          </div>
        </div>
      </div>

      {/* Strategy Table */}
      <div className="card strategy-table-card">
        <h3>Strategy Leaderboard</h3>

        <div className="strategy-table-wrapper">
          <table className="strategy-table">
            <thead>
              <tr>
                <th>Strategy</th>
                <th title="Viability Score (0-100)">Score</th>
                <th title="Win Rate Percentage">Win Rate</th>
                <th title="Profit Factor (Wins/Losses)">Profit Factor</th>
                <th title="Total Signals">Signals</th>
                <th title="Recommended Action">Action</th>
              </tr>
            </thead>
            <tbody>
              {sortedStrategies.slice(0, 12).map((strategy) => (
                <tr key={strategy.strategy_name} className="strategy-row">
                  <td className="strategy-name">
                    <span className="status-icon">{getStatusIcon(strategy.viability_score)}</span>
                    <span>{strategy.strategy_name}</span>
                  </td>
                  <td>
                    <span
                      className="score-badge"
                      style={{
                        backgroundColor: getStatusColor(strategy.viability_score),
                        color: 'white'
                      }}
                    >
                      {strategy.viability_score.toFixed(0)}
                    </span>
                  </td>
                  <td>
                    <span className="win-rate">
                      {(strategy.win_rate * 100).toFixed(0)}%
                    </span>
                  </td>
                  <td>
                    <span className="profit-factor">
                      {strategy.profit_factor.toFixed(2)}x
                    </span>
                  </td>
                  <td className="signal-count">{strategy.total_signals}</td>
                  <td>
                    <span
                      className="action-badge"
                      style={{
                        backgroundColor: getActionColor(strategy.action),
                        color: 'white'
                      }}
                    >
                      {strategy.action}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {/* Table Footer */}
        <div className="table-footer">
          <p className="footer-text">
            Showing top {Math.min(12, sortedStrategies.length)} of {strategies.length} strategies
          </p>
          <p className="footer-hint">
            • Score 85+: Excellent (increase weight)
            • Score 70-84: Good (use as-is)
            • Score 50-69: Monitor
            • Score &lt;50: Reduce or remove
          </p>
        </div>
      </div>

      {/* Strategy Categories */}
      <div className="card">
        <h3>Strategy Categories</h3>

        <div className="category-grid">
          {/* Excellent Strategies */}
          <div className="category-box excellent">
            <h4>🟢 Excellent (85+)</h4>
            <ul>
              {sortedStrategies
                .filter(s => s.viability_score >= 85)
                .map((s) => (
                  <li key={s.strategy_name}>
                    <span className="name">{s.strategy_name}</span>
                    <span className="score">{s.viability_score.toFixed(0)}</span>
                  </li>
                ))}
              {sortedStrategies.filter(s => s.viability_score >= 85).length === 0 && (
                <li className="no-data">No strategies in this category</li>
              )}
            </ul>
          </div>

          {/* Good Strategies */}
          <div className="category-box good">
            <h4>🟡 Good (70-84)</h4>
            <ul>
              {sortedStrategies
                .filter(s => s.viability_score >= 70 && s.viability_score < 85)
                .map((s) => (
                  <li key={s.strategy_name}>
                    <span className="name">{s.strategy_name}</span>
                    <span className="score">{s.viability_score.toFixed(0)}</span>
                  </li>
                ))}
              {sortedStrategies.filter(s => s.viability_score >= 70 && s.viability_score < 85).length === 0 && (
                <li className="no-data">No strategies in this category</li>
              )}
            </ul>
          </div>

          {/* Monitor Strategies */}
          <div className="category-box monitor">
            <h4>🟠 Monitor (50-69)</h4>
            <ul>
              {sortedStrategies
                .filter(s => s.viability_score >= 50 && s.viability_score < 70)
                .map((s) => (
                  <li key={s.strategy_name}>
                    <span className="name">{s.strategy_name}</span>
                    <span className="score">{s.viability_score.toFixed(0)}</span>
                  </li>
                ))}
              {sortedStrategies.filter(s => s.viability_score >= 50 && s.viability_score < 70).length === 0 && (
                <li className="no-data">No strategies in this category</li>
              )}
            </ul>
          </div>

          {/* Poor Strategies */}
          <div className="category-box poor">
            <h4>🔴 Remove (&lt;50)</h4>
            <ul>
              {sortedStrategies
                .filter(s => s.viability_score < 50)
                .map((s) => (
                  <li key={s.strategy_name}>
                    <span className="name">{s.strategy_name}</span>
                    <span className="score">{s.viability_score.toFixed(0)}</span>
                  </li>
                ))}
              {sortedStrategies.filter(s => s.viability_score < 50).length === 0 && (
                <li className="no-data">No strategies in this category</li>
              )}
            </ul>
          </div>
        </div>
      </div>

      {/* Metrics Explanation */}
      <div className="card metrics-explanation">
        <h3>📊 Understanding the Metrics</h3>

        <div className="metric-details">
          <div className="metric">
            <h5>Viability Score (0-100)</h5>
            <p>
              Overall performance rating combining win rate, profit factor, Sharpe ratio, and data quality.
              Score 85+ indicates an excellent strategy worth using and increasing weight.
            </p>
          </div>

          <div className="metric">
            <h5>Win Rate</h5>
            <p>
              Percentage of signals that resulted in winning trades. Target: 60%+ for crypto trading.
              75%+ win rate is excellent and rare.
            </p>
          </div>

          <div className="metric">
            <h5>Profit Factor</h5>
            <p>
              Total profits divided by total losses. Score of 2.0x means $2 profit for every $1 loss.
              Target: 1.5x+. Score below 1.0 means the strategy is losing.
            </p>
          </div>

          <div className="metric">
            <h5>Signals</h5>
            <p>
              Total number of times this strategy has signaled a trade.
              Minimum 30 signals recommended for reliable analysis.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
};

export default StrategyPerformanceDashboard;
