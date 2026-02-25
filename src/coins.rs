//! Coin metadata dictionary.
//!
//! Provides full names and logo URLs for all major cryptocurrencies.
//! Logo URLs use the TradingView CDN which follows a consistent pattern
//! and works without authentication or CORS issues.

/// Return the full display name for a ticker symbol.
/// Falls back to the symbol itself if not in the dictionary.
pub fn coin_name(symbol: &str) -> &'static str {
    match symbol.to_uppercase().as_str() {
        "BTC"   => "Bitcoin",
        "ETH"   => "Ethereum",
        "SOL"   => "Solana",
        "BNB"   => "BNB",
        "XRP"   => "XRP",
        "ADA"   => "Cardano",
        "AVAX"  => "Avalanche",
        "DOGE"  => "Dogecoin",
        "LINK"  => "Chainlink",
        "DOT"   => "Polkadot",
        "MATIC" => "Polygon",
        "POL"   => "Polygon",
        "LTC"   => "Litecoin",
        "BCH"   => "Bitcoin Cash",
        "ATOM"  => "Cosmos",
        "UNI"   => "Uniswap",
        "AAVE"  => "Aave",
        "OP"    => "Optimism",
        "ARB"   => "Arbitrum",
        "APT"   => "Aptos",
        "SUI"   => "Sui",
        "INJ"   => "Injective",
        "TIA"   => "Celestia",
        "PYTH"  => "Pyth Network",
        "WIF"   => "dogwifhat",
        "PEPE"  => "Pepe",
        "SHIB"  => "Shiba Inu",
        "BONK"  => "Bonk",
        "FLOKI" => "Floki",
        "WLD"   => "Worldcoin",
        "JUP"   => "Jupiter",
        "RNDR"  => "Render",
        "GRT"   => "The Graph",
        "FET"   => "Fetch.ai",
        "NEAR"  => "NEAR Protocol",
        "FTM"   => "Fantom",
        "S"     => "Sonic",
        "ALGO"  => "Algorand",
        "VET"   => "VeChain",
        "FIL"   => "Filecoin",
        "SAND"  => "The Sandbox",
        "MANA"  => "Decentraland",
        "AXS"   => "Axie Infinity",
        "CHZ"   => "Chiliz",
        "ENJ"   => "Enjin Coin",
        "GALA"  => "Gala",
        "IMX"   => "Immutable X",
        "LDO"   => "Lido DAO",
        "MKR"   => "Maker",
        "CRV"   => "Curve",
        "SNX"   => "Synthetix",
        "COMP"  => "Compound",
        "YFI"   => "Yearn Finance",
        "SUSHI" => "SushiSwap",
        "1INCH" => "1inch",
        "RPL"   => "Rocket Pool",
        "BAL"   => "Balancer",
        "ZEC"   => "Zcash",
        "XMR"   => "Monero",
        "EOS"   => "EOS",
        "XLM"   => "Stellar",
        "TRX"   => "TRON",
        "ICP"   => "Internet Computer",
        "XTZ"   => "Tezos",
        "HBAR"  => "Hedera",
        "FLOW"  => "Flow",
        "QNT"   => "Quant",
        "STX"   => "Stacks",
        "MINA"  => "Mina Protocol",
        "CFX"   => "Conflux",
        "KAVA"  => "Kava",
        "OSMO"  => "Osmosis",
        "SEI"   => "Sei",
        "BLUR"  => "Blur",
        "GMX"   => "GMX",
        "DYDX"  => "dYdX",
        "PERP"  => "Perpetual Protocol",
        "PENDLE"=> "Pendle",
        "STRK"  => "Starknet",
        "MANTA" => "Manta Network",
        "ALT"   => "AltLayer",
        "EIGEN" => "EigenLayer",
        "ENA"   => "Ethena",
        "IO"    => "io.net",
        "BOME"  => "BOOK OF MEME",
        "MEW"   => "cat in a dogs world",
        "TURBO" => "Turbo",
        "NEIRO" => "Neiro",
        "GOAT"  => "Goat",
        "PNUT"  => "Peanut the Squirrel",
        "ACT"   => "Act I : The AI Prophecy",
        "POPCAT"=> "Popcat",
        "NOT"   => "Notcoin",
        "DOGS"  => "Dogs",
        "BRETT" => "Brett",
        "TRUMP" => "Official Trump",
        "MELANIA"=> "Melania Meme",
        "VIRTUAL"=> "Virtuals Protocol",
        "AI16Z" => "ai16z",
        "AIXBT" => "AIXBT",
        "FARTCOIN"=>"Fartcoin",
        "HYPE"  => "Hyperliquid",
        "KAS"   => "Kaspa",
        "TON"   => "Toncoin",
        "JASMY" => "JasmyCoin",
        "TAO"   => "Bittensor",
        "CKB"   => "Nervos Network",
        "ZK"    => "ZKsync",
        "ZETA"  => "ZetaChain",
        "NTRN"  => "Neutron",
        "DYM"   => "Dymension",
        "W"     => "Wormhole",
        "BEAM"  => "Beam",
        "ONDO"  => "Ondo",
        "JTO"   => "Jito",
        "ORCA"  => "Orca",
        "RAY"   => "Raydium",
        "DRIFT" => "Drift",
        "KMNO"  => "Kamino",
        _       => "",   // empty string = caller falls back to ticker
    }
}

/// Return a logo `<img>` tag for a coin symbol.
///
/// Uses TradingView's public CDN which has consistent URLs for all
/// major cryptocurrencies and serves SVGs for crisp display at any size.
/// `size` controls the CSS width/height in pixels.
///
/// Falls back gracefully via `onerror` to an emoji placeholder.
pub fn coin_logo_img(symbol: &str, size: u32) -> String {
    let sym_upper = symbol.to_uppercase();
    // TradingView CDN: consistent for all major coins.
    let tv_url = format!(
        "https://s3-symbol-logo.tradingview.com/crypto/XTVC{}--big.svg",
        sym_upper
    );
    // CoinCap fallback (PNG, lower-case)
    let cc_url = format!(
        "https://assets.coincap.io/assets/icons/{}@2x.png",
        symbol.to_lowercase()
    );
    format!(
        r#"<img src="{tv}" onerror="this.onerror=null;this.src='{cc}'"
             width="{sz}" height="{sz}"
             style="border-radius:50%;vertical-align:middle;margin-right:5px"
             alt="{sym}">"#,
        tv  = tv_url,
        cc  = cc_url,
        sz  = size,
        sym = sym_upper,
    )
}
