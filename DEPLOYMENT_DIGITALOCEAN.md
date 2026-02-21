# DigitalOcean Deployment Guide

## Machine Specs

**Minimum Requirements:**
- Droplet: Basic ($6-12/month)
- CPU: 1 vCPU
- RAM: 1GB
- Storage: 25GB SSD
- OS: Ubuntu 22.04 LTS

**Recommended:**
- Droplet: Standard ($12-24/month)
- CPU: 2 vCPU
- RAM: 2GB
- Storage: 60GB SSD
- OS: Ubuntu 22.04 LTS

## Deployment Steps

### 1. Create Droplet

```bash
# Via DigitalOcean Console:
# 1. Click "Create" → "Droplets"
# 2. Choose Ubuntu 22.04 LTS
# 3. Select Standard Droplet ($12/month)
# 4. Select 2 vCPU / 2GB RAM / 60GB SSD
# 5. Add SSH key
# 6. Create
```

### 2. SSH into VPS

```bash
ssh root@your_droplet_ip
```

### 3. Update System

```bash
apt update && apt upgrade -y
```

### 4. Install Docker & Docker Compose

```bash
# Install Docker
curl -fsSL https://get.docker.com -o get-docker.sh
sudo sh get-docker.sh

# Install Docker Compose
sudo curl -L "https://github.com/docker/compose/releases/latest/download/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
sudo chmod +x /usr/local/bin/docker-compose

# Verify
docker --version
docker-compose --version
```

### 5. Clone Repository

```bash
cd /root
git clone https://github.com/yourusername/redrobot-hedgebot.git
cd redrobot-hedgebot
```

### 6. Configure Environment

```bash
# Create production .env
cat > .env << 'ENVEOF'
MODE=testnet
TRADING_SYMBOL=SOL
INITIAL_CAPITAL=100
BINANCE_API_KEY=your_actual_key
HYPERLIQUID_KEY=your_actual_key
HYPERLIQUID_SECRET=your_actual_secret
DATABASE_URL=postgres://postgres:postgres@postgres:5432/redrobot
RUST_LOG=info
ENVEOF
```

### 7. Deploy with Docker Compose

```bash
# Start services
docker-compose up -d

# Check logs
docker-compose logs -f redrobot

# Check database
docker-compose exec postgres psql -U postgres -d redrobot -c "\dt"
```

### 8. Monitor System

```bash
# View running containers
docker ps

# Check resource usage
docker stats

# View application logs
docker-compose logs -f

# Health check
curl http://localhost:8080/health
```

### 9. Switch to Mainnet (After Validation)

```bash
# Edit .env
sed -i 's/MODE=testnet/MODE=mainnet/' .env

# Restart
docker-compose restart redrobot

# Watch logs
docker-compose logs -f redrobot
```

### 10. Setup Automatic Backups

```bash
# Create backup script
cat > /root/backup-database.sh << 'BKEOF'
#!/bin/bash
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
docker-compose exec -T postgres pg_dump -U postgres redrobot > /root/backups/redrobot_${TIMESTAMP}.sql
find /root/backups -name "redrobot_*.sql" -mtime +7 -delete
BKEOF

chmod +x /root/backup-database.sh

# Add to crontab
(crontab -l 2>/dev/null; echo "0 2 * * * /root/backup-database.sh") | crontab -
```

## Firewall Setup

```bash
# Allow SSH
ufw allow 22/tcp

# Allow HTTP (for monitoring dashboard)
ufw allow 8080/tcp

# Block everything else
ufw default deny incoming
ufw enable
```

## Monitoring & Alerts

### System Monitoring
```bash
# Install monitoring agent
curl -sSL https://agent.digitalocean.com/install.sh | sh

# Enable monitoring
apt install sysstat -y
```

### Application Monitoring
```bash
# Create monitoring script
cat > /root/monitor.sh << 'MONEOF'
#!/bin/bash
while true; do
  if ! docker ps | grep redrobot; then
    docker-compose up -d
    echo "Restarted redrobot" | mail -s "RedRobot Alert" your_email@example.com
  fi
  sleep 300
done
MONEOF

nohup bash /root/monitor.sh &
```

## Troubleshooting

### Issue: Docker service won't start
```bash
systemctl restart docker
docker ps  # Should work now
```

### Issue: Database connection error
```bash
# Check PostgreSQL is running
docker-compose ps postgres

# Reset database
docker-compose down
docker volume rm redrobot-hedgebot_postgres_data
docker-compose up -d
```

### Issue: High memory usage
```bash
# Check container stats
docker stats

# Restart if needed
docker-compose restart redrobot
```

## Security Best Practices

1. **Rotate API keys regularly**
   ```bash
   # Update .env with new keys
   docker-compose restart redrobot
   ```

2. **Use VPN for connections**
   ```bash
   # SSH with public key only
   # Disable password login in /etc/ssh/sshd_config
   ```

3. **Monitor file permissions**
   ```bash
   # Protect .env
   chmod 600 .env
   ```

4. **Daily backups**
   - Automated via cron (see Backup section)
   - Test restore procedures monthly

## Cost Breakdown (Monthly)

- DigitalOcean Droplet: $12-24
- Data transfer: ~$0 (included)
- PostgreSQL local: $0
- External APIs: $0-20 (optional)
- **Total: $12-44/month**

## Expected Uptime

- 99.9% with proper monitoring
- Auto-restart on failure
- Database redundancy available

## Scaling to Larger Capital

For trading $1K+:

1. **Upgrade to App Platform** ($12-100/month)
2. **Add managed PostgreSQL** ($15-50/month)
3. **Add monitoring service** ($5/month)
4. **Multiple regions** for redundancy

**Total scaling cost: $50-200/month**

