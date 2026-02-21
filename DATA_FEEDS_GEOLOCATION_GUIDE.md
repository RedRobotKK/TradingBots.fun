# 🌍 Data Feeds Geolocation Guide: API Availability by Region

**Purpose:** Document geographic restrictions for each data API and provide deployment guidance
**Status:** Critical for production deployment
**Impact:** Determines which APIs work in your deployment region

---

## 🚨 **Critical Reality: Geofencing & Regional Restrictions**

Many cryptocurrency APIs have geographic restrictions based on:
- User's location
- Bot's server location
- IP address geolocation
- Regulatory compliance (GDPR, sanctions, etc.)

**Key Insight:** The bot's SERVER LOCATION matters more than user location.

---

## 📍 **API Availability by Region**

### **1. BINANCE API**

```
🟢 AVAILABLE IN:
  ✅ Asia (Primary - fully supported)
  ✅ Europe (Supported)
  ✅ Americas (Supported)
  ✅ Africa (Supported)
  ✅ Middle East (Supported)

🔴 RESTRICTED IN:
  ❌ United States (US traders can't use mainnet)
  ❌ Some US states (even within US)
  ❌ Canada (some restrictions)
  ❌ Iran, Syria, North Korea (sanctions)
  ❌ Crimea (sanctions)

🟡 PARTIALLY RESTRICTED:
  ⚠️ Hong Kong (some features limited)
  ⚠️ Singapore (some features limited)
  ⚠️ Japan (regulated, needs local entity)
  ⚠️ South Korea (regulated)

RECOMMENDATION FOR DEPLOYMENT:
  🏆 BEST: Asia (Singapore, Japan, Hong Kong) - No restrictions
  ✅ GOOD: Europe (UK, EU) - Full access
  ❌ AVOID: USA - Use alternative APIs instead

WORKAROUND IF IN USA:
  Option 1: VPN to Asia
  Option 2: Use Kraken instead
  Option 3: Use CoinGecko instead
  Option 4: Use Hyperliquid (no geofencing)
```

**Test Your Access:**
```bash
# Test if Binance is accessible from your location
curl -I https://api.binance.com/api/v3/ping
# If you get 200 OK: ✅ Access is good
# If you get 403/404: ❌ Geofenced
```

---

### **2. COINGECKO API**

```
🟢 AVAILABLE IN:
  ✅ Worldwide (NO geofencing!)
  ✅ USA (fully available)
  ✅ Canada (fully available)
  ✅ Europe (fully available)
  ✅ Asia (fully available)
  ✅ Africa (fully available)

🔴 RESTRICTED IN:
  ❌ None (no geofencing)
  ❌ Only sanctioned countries (same as everywhere)

RECOMMENDATION FOR DEPLOYMENT:
  🏆 BEST: Anywhere (no geofencing!)
  ✅ FALLBACK: Always use this as backup
  ✅ WORLDWIDE: Safe choice for global deployment

KEY ADVANTAGE:
  - No IP-based geofencing
  - No VPN needed
  - Works everywhere
  - Limited rate (50 req/min) but reliable
  - Perfect as fallback
```

**Test Your Access:**
```bash
curl https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd
# Should work from anywhere!
```

---

### **3. KRAKEN API**

```
🟢 AVAILABLE IN:
  ✅ Most countries (no strict geofencing)
  ✅ USA (public API available)
  ✅ Europe (available)
  ✅ Asia (available)
  ✅ Canada (available)

🟡 PARTIALLY AVAILABLE:
  ⚠️ USA (can't trade, but can use public API)
  ⚠️ Some countries need VPN (check local laws)

🔴 RESTRICTED IN:
  ❌ Sanctioned countries
  ❌ Certain high-risk jurisdictions

RECOMMENDATION FOR DEPLOYMENT:
  🏆 BEST: Europe (fully available)
  ✅ GOOD: Anywhere outside USA (no issues)
  ⚠️ USA: Public API works, but trading restricted

KEY ADVANTAGE:
  - Higher rate limit than CoinGecko (900 req/min)
  - No geofencing for public API
  - Reliable endpoint
  - Good fallback
```

**Test Your Access:**
```bash
curl https://api.kraken.com/0/public/Ticker?pair=XSOLZUSD
# Should work from most locations
```

---

### **4. HYPERLIQUID API**

```
🟢 AVAILABLE IN:
  ✅ Worldwide (NO geofencing!)
  ✅ USA (fully available)
  ✅ Europe (fully available)
  ✅ Asia (fully available)
  ✅ Works from anywhere

🔴 RESTRICTED IN:
  ❌ None (DEX - fully decentralized)
  ❌ Only sanctioned addresses blocked

RECOMMENDATION FOR DEPLOYMENT:
  🏆 BEST: Your primary venue
  ✅ EVERYWHERE: Works from any location
  ✅ FALLBACK: Perfect for geographic redundancy

KEY ADVANTAGE:
  - No geofencing (DEX advantage)
  - Direct venue pricing
  - Unlimited rate limit
  - Most current prices for your trades
  - Works globally
```

**Test Your Access:**
```bash
curl https://api.hyperliquid.com/info -X POST \
  -H "Content-Type: application/json" \
  -d '{"type":"metaAndAssetCtxs"}'
# Should work from anywhere!
```

---

### **5. POLYGON.IO API**

```
🟢 AVAILABLE IN:
  ✅ USA (primary market)
  ✅ Most countries (decent coverage)

🟡 PARTIALLY AVAILABLE:
  ⚠️ Europe (GDPR restrictions)
  ⚠️ Canada (some features limited)
  ⚠️ Asia (some restrictions)

🔴 RESTRICTED IN:
  ❌ GDPR countries (EU) - strict data policies
  ❌ Some countries require local setup

RECOMMENDATION FOR DEPLOYMENT:
  ⚠️ AVOID for primary source (too restrictive)
  ✅ OPTIONAL: Only if you specifically need their data
  ❌ EUROPE: Don't use (GDPR issues)
  ✅ USA: Can use if needed

KEY LIMITATION:
  - Geofenced API
  - Requires authentication
  - Rate limited (5 req/min free)
  - Not recommended for global deployment
```

**Test Your Access:**
```bash
curl "https://api.polygon.io/v1/last/crypto?cryptoticker=CSOL&apikey=YOUR_KEY"
# May fail if you're outside USA/supported regions
```

---

## 🗺️ **Recommended Setup by Region**

### **Region: USA**

```
PRIMARY:     CoinGecko (no geofencing)
FALLBACK 1:  Kraken (public API works)
FALLBACK 2:  Hyperliquid (DEX, no geo)
AVOID:       Binance (geofenced)

DEPLOYMENT:
  ✅ Use VPN to Asia if you want Binance
  ✅ Or just use CoinGecko (works fine)
  ✅ Hyperliquid is your venue anyway
```

### **Region: Europe (Non-GDPR Focus)**

```
PRIMARY:     Binance (works well)
FALLBACK 1:  Kraken (excellent coverage)
FALLBACK 2:  CoinGecko (backup)
VENUE:       Hyperliquid (always works)

DEPLOYMENT:
  ✅ All major APIs available
  ✅ No special configuration needed
  ✅ Good geographic diversity
```

### **Region: Europe (GDPR-Strict)**

```
PRIMARY:     CoinGecko (no GDPR issues)
FALLBACK 1:  Kraken (GDPR compliant)
FALLBACK 2:  Hyperliquid (DEX, no GDPR)
AVOID:       Polygon.io (GDPR restricted)

DEPLOYMENT:
  ✅ Use privacy-first APIs
  ✅ CoinGecko + Kraken is solid combo
  ✅ No personal data collected
```

### **Region: Asia**

```
PRIMARY:     Binance (best in Asia)
FALLBACK 1:  CoinGecko (no restrictions)
FALLBACK 2:  Kraken (available)
VENUE:       Hyperliquid (works globally)

DEPLOYMENT:
  ✅ Binance is optimal here
  ✅ No geofencing issues
  ✅ Best rates and coverage
```

### **Region: Rest of World**

```
PRIMARY:     Hyperliquid (DEX, no geo)
FALLBACK 1:  CoinGecko (worldwide)
FALLBACK 2:  Kraken (good coverage)
OPTIONAL:    Binance (check if available)

DEPLOYMENT:
  ✅ CoinGecko + Hyperliquid = solid backup
  ✅ Check Binance/Kraken for your country
  ✅ DEX advantage (Hyperliquid)
```

---

## 🔧 **User-Configurable Data Feeds Architecture**

### **Design Principle: User Can Customize**

```
Users should be able to:
  ✅ Enable/disable APIs per their location
  ✅ Set custom priority order
  ✅ Add their own API endpoints
  ✅ Configure timeouts per API
  ✅ Test connectivity before deployment
```

### **Configuration File Structure**

```env
# .env.data-feeds.example

# ============================================================================
# DATA FEEDS - USER CUSTOMIZABLE
# ============================================================================

# Which APIs are enabled for YOUR location?
# (Customize based on your geographic region)

BINANCE_ENABLED=true              # Set to false if geofenced in your location
COINGECKO_ENABLED=true            # Safe everywhere
KRAKEN_ENABLED=true               # Safe most places
HYPERLIQUID_ENABLED=true          # Safe everywhere (your venue)
POLYGON_ENABLED=false             # Only enable if you need it + can access

# ============================================================================
# PRIORITY ORDER - User sets based on their setup
# ============================================================================

# Which API to try first?
# Options: binance, coingecko, kraken, hyperliquid
PRIMARY_DATA_SOURCE=binance

# If primary fails, try this
FALLBACK_DATA_SOURCE=coingecko

# If both fail, try this
SECONDARY_FALLBACK=kraken

# Your trading venue (always try)
VENUE_DATA_SOURCE=hyperliquid

# ============================================================================
# GEOGRAPHIC CONSIDERATIONS
# ============================================================================

# Where is your bot deployed?
DEPLOYMENT_REGION=asia            # asia, europe, usa, other
REQUIRES_VPN=false                # Set true if using VPN to bypass geo

# Test API access on startup?
TEST_API_CONNECTIVITY=true        # Fails gracefully if API unavailable

# ============================================================================
# CUSTOM API ENDPOINTS (For power users)
# ============================================================================

# Add your own APIs here (optional)
# Format: CUSTOM_API_1_URL, CUSTOM_API_1_ENABLED, etc

CUSTOM_API_1_URL=                 # Leave blank unless adding custom API
CUSTOM_API_1_ENABLED=false
CUSTOM_API_1_RATE_LIMIT=0

# ============================================================================
# GEOLOCATION WORKAROUNDS
# ============================================================================

# If Binance is geofenced in your location:
BINANCE_VPN_ENABLED=false         # Set true if using VPN
BINANCE_PROXY_URL=                # Optional: proxy server URL

# If you need to rotate IPs:
ROTATE_IPS=false                  # Set true if using IP rotation service
IP_ROTATION_SERVICE=              # Leave blank unless using service
```

---

## 🧪 **Testing Your API Availability**

### **Script: Test All APIs Before Deployment**

```bash
#!/bin/bash
# test_api_availability.sh

echo "Testing Data Feed Availability..."
echo "================================="

# Test Binance
echo -n "Binance: "
if curl -s -I https://api.binance.com/api/v3/ping | grep -q "200"; then
    echo "✅ AVAILABLE"
else
    echo "❌ GEOFENCED"
fi

# Test CoinGecko
echo -n "CoinGecko: "
if curl -s https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd | grep -q "bitcoin"; then
    echo "✅ AVAILABLE"
else
    echo "❌ BLOCKED"
fi

# Test Kraken
echo -n "Kraken: "
if curl -s https://api.kraken.com/0/public/Ticker?pair=XBTUSDT | grep -q "XBTUSDT"; then
    echo "✅ AVAILABLE"
else
    echo "❌ BLOCKED"
fi

# Test Hyperliquid
echo -n "Hyperliquid: "
if curl -s https://api.hyperliquid.com/info -X POST | grep -q "metaAndAssetCtxs"; then
    echo "✅ AVAILABLE"
else
    echo "❌ BLOCKED"
fi

echo ""
echo "Configure your .env.data-feeds based on results above"
```

---

## 🌐 **VPN/Proxy Solutions for Geofenced APIs**

### **If You're in a Geofenced Region (e.g., USA wanting Binance)**

```
Option 1: VPN to Asia
  ✅ Simplest solution
  ✅ Route bot traffic through VPN
  ✅ Binance becomes available
  ⚠️  May violate ToS (check Binance)
  📍 VPN to: Singapore, Japan, Hong Kong

Option 2: Proxy Service
  ✅ More sophisticated than VPN
  ✅ Some services specialize in API access
  ⚠️  Adds latency
  ⚠️  May violate ToS

Option 3: Use Alternative APIs
  ✅ BEST: Just use CoinGecko + Kraken
  ✅ Legal in all jurisdictions
  ✅ No ToS violations
  ✅ Still get accurate data
  ✅ Slightly less rate limit
  ⚠️  Minor data freshness difference

RECOMMENDATION:
  🏆 BEST: Option 3 (use alternatives)
  ✅ ACCEPTABLE: Option 1 (VPN to Asia)
  ❌ AVOID: Option 2 (proxy - slower)
```

---

## 📝 **Deployment Documentation Template**

### **Users Must Document Their Setup:**

```
Before deploying, fill this out:

DEPLOYMENT REGION: _______________
(usa, europe, asia, other)

BINANCE AVAILABLE: Yes / No
COINGECKO AVAILABLE: Yes / No
KRAKEN AVAILABLE: Yes / No
HYPERLIQUID AVAILABLE: Yes / No

USING VPN: Yes / No
VPN LOCATION: _______________

PRIMARY API: _______________
FALLBACK API: _______________

EXPECTED ISSUES: _______________
MITIGATION STRATEGY: _______________

Date: _______________
Tested By: _______________
```

---

## 🔐 **Legal & Compliance Notes**

### **Important Considerations**

```
1. GEOGRAPHIC RESTRICTIONS ARE LEGAL
   ✅ Binance restricts USA (their choice)
   ✅ GDPR restricts EU data handling
   ✅ Sanctions restrict certain countries
   ✅ These are legal requirements

2. VPN USAGE LEGAL GRAY AREA
   ⚠️  Using VPN to bypass geo restrictions:
   - May violate API ToS
   - May violate local law
   - Varies by jurisdiction
   - Consult legal counsel if unsure
   - SAFER: Use alternative APIs

3. BEST PRACTICE
   ✅ Always use alternative APIs first
   ✅ Only use VPN if absolutely necessary
   ✅ Document your compliance approach
   ✅ Consider regional deployment location
   ✅ Test APIs before deployment

4. RECOMMENDATION FOR USERS
   💡 If in USA:
      - Use CoinGecko (works fine)
      - Or deploy bot in Asia
      - Or use VPN (at your risk)

   💡 If in Europe:
      - Use CoinGecko (GDPR safe)
      - Or use Kraken
      - No geofencing issues

   💡 If in Asia:
      - Use Binance (optimal)
      - All APIs available
      - Best rates and speed
```

---

## ✅ **Deployment Checklist**

```
Before deploying your bot:

[ ] Tested all APIs from your location
[ ] Documented which APIs work
[ ] Configured .env.data-feeds accordingly
[ ] Set PRIMARY_DATA_SOURCE to working API
[ ] Set FALLBACK_DATA_SOURCE for redundancy
[ ] DEPLOYMENT_REGION set correctly
[ ] Understand legal implications
[ ] Documented your setup
[ ] Ready to deploy

If any APIs blocked:
[ ] Decided: VPN, Proxy, or Alternative APIs
[ ] Tested workaround before deployment
[ ] Updated configuration files
[ ] Documented the workaround
[ ] Ready to deploy
```

---

## 📊 **API Availability Matrix**

| API | USA | Europe | Asia | GDPR | Cost | Rate Limit | Geo |
|-----|-----|--------|------|------|------|-----------|-----|
| **Binance** | ❌ | ✅ | ✅✅ | N/A | $0 | 1200/min | 🔴 Yes |
| **CoinGecko** | ✅✅ | ✅✅ | ✅✅ | ✅ | $0 | 50/min | 🟢 No |
| **Kraken** | ✅ | ✅✅ | ✅ | ✅ | $0 | 900/min | 🟡 Minor |
| **Hyperliquid** | ✅✅ | ✅✅ | ✅✅ | ✅ | $0 | ∞ | 🟢 No |
| **Polygon** | ✅ | ❌ | ⚠️ | ❌ | $0 | 5/min | 🔴 Yes |

**Legend:**
- 🟢 No geofencing
- 🟡 Minor restrictions
- 🔴 Significant restrictions

---

## 🚀 **Implementation Code Example**

```rust
// src/config/geolocation.rs

#[derive(Debug, Clone)]
pub struct GeolocationConfig {
    pub deployment_region: DeploymentRegion,
    pub enabled_apis: Vec<ApiProvider>,
    pub primary_source: ApiProvider,
    pub fallback_sources: Vec<ApiProvider>,
    pub uses_vpn: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentRegion {
    USA,
    Europe,
    Asia,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiProvider {
    Binance,
    CoinGecko,
    Kraken,
    Hyperliquid,
    Polygon,
}

impl GeolocationConfig {
    pub fn from_env() -> Self {
        let region = match std::env::var("DEPLOYMENT_REGION").as_deref() {
            Ok("usa") => DeploymentRegion::USA,
            Ok("europe") => DeploymentRegion::Europe,
            Ok("asia") => DeploymentRegion::Asia,
            _ => DeploymentRegion::Other,
        };

        let enabled_apis = vec![
            if std::env::var("BINANCE_ENABLED")
                .unwrap_or_default()
                .parse::<bool>()
                .unwrap_or(true)
            {
                Some(ApiProvider::Binance)
            } else {
                None
            },
            if std::env::var("COINGECKO_ENABLED")
                .unwrap_or_default()
                .parse::<bool>()
                .unwrap_or(true)
            {
                Some(ApiProvider::CoinGecko)
            } else {
                None
            },
            // ... more providers
        ]
        .into_iter()
        .flatten()
        .collect();

        Self {
            deployment_region: region,
            enabled_apis,
            primary_source: ApiProvider::CoinGecko, // Safe default
            fallback_sources: vec![ApiProvider::Kraken, ApiProvider::Hyperliquid],
            uses_vpn: std::env::var("REQUIRES_VPN")
                .unwrap_or_default()
                .parse::<bool>()
                .unwrap_or(false),
        }
    }

    pub fn can_use_binance(&self) -> bool {
        !self.enabled_apis.contains(&ApiProvider::Binance)
            || (self.uses_vpn && self.deployment_region == DeploymentRegion::USA)
    }

    pub fn recommended_fallback_chain(&self) -> Vec<ApiProvider> {
        match self.deployment_region {
            DeploymentRegion::USA => {
                vec![ApiProvider::CoinGecko, ApiProvider::Kraken, ApiProvider::Hyperliquid]
            }
            DeploymentRegion::Europe => {
                vec![ApiProvider::Binance, ApiProvider::Kraken, ApiProvider::CoinGecko]
            }
            DeploymentRegion::Asia => {
                vec![ApiProvider::Binance, ApiProvider::CoinGecko, ApiProvider::Kraken]
            }
            DeploymentRegion::Other => {
                vec![ApiProvider::CoinGecko, ApiProvider::Hyperliquid, ApiProvider::Kraken]
            }
        }
    }
}
```

---

**Status:** Complete guide for geolocation-aware deployment
**Next:** Users must test APIs and configure accordingly
**Critical:** Document your setup before deploying to mainnet

