# GitHub Setup & Deployment

## Create GitHub Repository

### Option 1: Via GitHub Web UI

1. Go to https://github.com/new
2. Name: `tradingbots-fun`
3. Description: "Autonomous cryptocurrency trading bot for Solana DEX"
4. Make it **PRIVATE** (don't expose API keys)
5. Click "Create repository"

### Option 2: Via GitHub CLI

```bash
# Install gh CLI
brew install gh  # macOS
# Or: curl -fsSL https://cli.github.com/install.sh | sh  # Linux

# Login
gh auth login

# Create repo
gh repo create tradingbots-fun --private --source=. --remote=origin --push
```

## Push Code to GitHub

### First Time Setup

```bash
# Navigate to project
cd /sessions/confident-eloquent-wozniak/mnt/Development/tradingbots-fun

# Initialize git (if not already done)
git init
git add .
git commit -m "Initial commit: Core trading system with Rust backend"

# Add remote
git remote add origin https://github.com/yourusername/tradingbots-fun.git

# Push to main
git branch -M main
git push -u origin main
```

### Update Code

```bash
# Make changes
# ... edit files ...

# Stage changes
git add .

# Commit
git commit -m "Feature: Add whale detection"

# Push
git push origin main
```

## GitHub Actions (Continuous Integration)

### Create GitHub Actions Workflow

```bash
mkdir -p .github/workflows
```

```yaml
# .github/workflows/test.yml
name: Test & Build

on:
  push:
    branches: [ main ]
  pull_request:
    branches: [ main ]

jobs:
  test:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_PASSWORD: postgres
          POSTGRES_DB: tradingbots
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
        ports:
          - 5432:5432

    steps:
    - uses: actions/checkout@v3

    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable

    - name: Cache cargo registry
      uses: actions/cache@v3
      with:
        path: ~/.cargo/registry
        key: ${{ runner.os }}-cargo-registry-${{ hashFiles('**/Cargo.lock') }}

    - name: Cache cargo index
      uses: actions/cache@v3
      with:
        path: ~/.cargo/git
        key: ${{ runner.os }}-cargo-git-${{ hashFiles('**/Cargo.lock') }}

    - name: Cache cargo build
      uses: actions/cache@v3
      with:
        path: target
        key: ${{ runner.os }}-cargo-build-target-${{ hashFiles('**/Cargo.lock') }}

    - name: Run tests
      run: cargo test --verbose
      env:
        DATABASE_URL: postgres://postgres:postgres@localhost:5432/tradingbots

    - name: Build release
      run: cargo build --release --verbose
```

Create file: `.github/workflows/test.yml`

## Secrets Management

### Add API Keys to GitHub Secrets

```bash
# Via CLI
gh secret set BINANCE_API_KEY -b "your_key_here"
gh secret set HYPERLIQUID_KEY -b "your_key_here"
gh secret set HYPERLIQUID_SECRET -b "your_secret_here"

# Or via GitHub Web UI:
# Settings → Secrets and variables → Actions → New repository secret
```

### Use Secrets in Workflows

```yaml
env:
  BINANCE_API_KEY: ${{ secrets.BINANCE_API_KEY }}
  HYPERLIQUID_KEY: ${{ secrets.HYPERLIQUID_KEY }}
```

## Repository Structure

```
tradingbots-fun/
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── data.rs
│   ├── indicators.rs
│   ├── signals.rs
│   ├── risk.rs
│   ├── exchange.rs
│   ├── decision.rs
│   ├── db.rs
│   └── monitoring.rs
├── migrations/
│   └── init.sql
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml
├── .env.example
├── .gitignore
├── README_PRODUCTION.md
├── DEPLOYMENT_DIGITALOCEAN.md
└── GITHUB_SETUP.md
```

## Keeping Repository Updated

### Daily Workflow

```bash
# Before starting work
git pull origin main

# Make changes
# ... editing ...

# Test locally
cargo build
cargo test

# Commit & push
git add .
git commit -m "Feature: Description"
git push origin main
```

### Weekly Backup

```bash
# Push to backup branch
git branch backup-$(date +%Y%m%d)
git push origin backup-$(date +%Y%m%d)
```

## Syncing from DigitalOcean

### Pull Latest on VPS

```bash
ssh root@your_vps_ip

# In /root/tradingbots-fun
cd tradingbots-fun
git pull origin main

# Rebuild if needed
docker-compose build
docker-compose restart tradingbots
```

## Collaboration (If Adding Team)

### Add Collaborators

1. Go to Repository Settings
2. Click "Collaborators"
3. Add team members

### Branch Protection Rules

1. Settings → Branches
2. Add rule for `main`
3. Require pull request reviews
4. Require status checks to pass

## Monitoring Repository

### GitHub Actions Status

- Go to Actions tab
- See all workflow runs
- Check logs for failures

### Code Quality

- Enable CodeQL (free security scanning)
- Settings → Security → Code security
- Enable "Dependabot alerts"

## Cloning on DigitalOcean

```bash
# On your local machine, push first
git push origin main

# Then on DigitalOcean
ssh root@your_vps_ip
cd /root
git clone https://github.com/yourusername/tradingbots-fun.git
cd tradingbots-fun
docker-compose up -d
```

---

**Next:** Deploy on DigitalOcean using `DEPLOYMENT_DIGITALOCEAN.md`

