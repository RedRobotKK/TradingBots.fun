#!/bin/bash
# RedRobot-HedgeBot Testnet Monitoring Script
# Run this to monitor the 72-hour testnet validation

set -e

LOG_FILE="/tmp/redrobot_testnet.log"
METRICS_FILE="/tmp/redrobot_metrics.json"
REPORT_FILE="/tmp/redrobot_testnet_report.txt"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}    RedRobot-HedgeBot TESTNET MONITORING DASHBOARD${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
echo ""

# Check if bot is running
if pgrep -f "target/release/redrobot" > /dev/null; then
    echo -e "${GREEN}✅ Bot Status: RUNNING${NC}"
    echo -e "   PID: $(pgrep -f 'target/release/redrobot')"
else
    echo -e "${RED}❌ Bot Status: NOT RUNNING${NC}"
    echo -e "   Start with: source .env.testnet && ./target/release/redrobot &"
fi

echo ""

# Check log file
if [ -f "$LOG_FILE" ]; then
    echo -e "${BLUE}📊 Recent Log Activity (Last 20 lines):${NC}"
    tail -20 "$LOG_FILE" | sed 's/^/   /'
else
    echo -e "${YELLOW}⚠️  Log file not found: $LOG_FILE${NC}"
fi

echo ""

# Count trades
if [ -f "$LOG_FILE" ]; then
    TRADE_COUNT=$(grep -c "Trade executed\|Order placed" "$LOG_FILE" 2>/dev/null || echo "0")
    echo -e "${BLUE}📈 Trade Statistics:${NC}"
    echo -e "   Total trades executed: ${GREEN}$TRADE_COUNT${NC}"

    # Calculate win rate
    WIN_COUNT=$(grep -c "Trade closed: PROFITABLE\|WIN" "$LOG_FILE" 2>/dev/null || echo "0")
    if [ $TRADE_COUNT -gt 0 ]; then
        WIN_RATE=$((WIN_COUNT * 100 / TRADE_COUNT))
        echo -e "   Winning trades: ${GREEN}$WIN_COUNT${NC}"
        echo -e "   Win rate: ${GREEN}$WIN_RATE%${NC}"
    fi
fi

echo ""

# Check errors
if [ -f "$LOG_FILE" ]; then
    ERROR_COUNT=$(grep -c "ERROR\|error\|PANIC" "$LOG_FILE" 2>/dev/null || echo "0")
    if [ $ERROR_COUNT -eq 0 ]; then
        echo -e "${GREEN}✅ Errors: NONE${NC}"
    else
        echo -e "${RED}⚠️  Errors detected: $ERROR_COUNT${NC}"
        echo -e "   Last 5 errors:"
        grep "ERROR\|error\|PANIC" "$LOG_FILE" 2>/dev/null | tail -5 | sed 's/^/   /'
    fi
fi

echo ""

# System resources
echo -e "${BLUE}💻 System Resources:${NC}"
if pgrep -f "target/release/redrobot" > /dev/null; then
    PS_INFO=$(ps aux | grep "target/release/redrobot" | grep -v grep)
    CPU=$(echo $PS_INFO | awk '{print $3}')
    MEM=$(echo $PS_INFO | awk '{print $4}')
    echo -e "   CPU Usage: ${GREEN}$CPU%${NC}"
    echo -e "   Memory Usage: ${GREEN}$MEM%${NC}"
fi

echo ""

# Uptime
if [ -f "$LOG_FILE" ]; then
    FIRST_LOG=$(head -1 "$LOG_FILE")
    echo -e "${BLUE}⏱️  Runtime:${NC}"
    echo -e "   Started: $FIRST_LOG"
fi

echo ""

# Health check status
if [ -f "$LOG_FILE" ]; then
    LAST_HEALTH=$(grep "Health check\|Account health" "$LOG_FILE" 2>/dev/null | tail -1 || echo "No health checks yet")
    echo -e "${BLUE}🏥 Latest Health Check:${NC}"
    echo -e "   $LAST_HEALTH" | sed 's/^/   /'
fi

echo ""

# Summary
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}VALIDATION CHECKLIST:${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"

# Check each criterion
if pgrep -f "target/release/redrobot" > /dev/null; then
    echo -e "${GREEN}✅${NC} Bot is running"
else
    echo -e "${RED}❌${NC} Bot is not running"
fi

if [ -f "$LOG_FILE" ]; then
    if grep -q "Autonomous runner started\|trading active" "$LOG_FILE" 2>/dev/null; then
        echo -e "${GREEN}✅${NC} Autonomous trading started"
    else
        echo -e "${YELLOW}⏳${NC} Waiting for autonomous trading to start"
    fi
fi

if [ -f "$LOG_FILE" ] && [ $ERROR_COUNT -lt 10 ]; then
    echo -e "${GREEN}✅${NC} Minimal errors (<10)"
else
    echo -e "${YELLOW}⚠️${NC}  Review errors in log"
fi

if [ -f "$LOG_FILE" ] && [ $TRADE_COUNT -gt 50 ]; then
    echo -e "${GREEN}✅${NC} Trading activity confirmed (>50 trades)"
elif [ $TRADE_COUNT -gt 0 ]; then
    echo -e "${YELLOW}⏳${NC} Trading in progress ($TRADE_COUNT trades)"
else
    echo -e "${YELLOW}⏳${NC} Waiting for first trades"
fi

echo ""
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}Next steps:${NC}"
echo -e "   1. Let the bot run for 72 hours continuously"
echo -e "   2. Run this script periodically: ./scripts/monitor-testnet.sh"
echo -e "   3. Check logs: tail -f /tmp/redrobot_testnet.log"
echo -e "   4. After 72 hours, proceed to mainnet deployment"
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
