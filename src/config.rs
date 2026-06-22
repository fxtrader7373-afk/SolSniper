use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub rpc_http_url: String,
    pub rpc_ws_url: String,
    pub wallet_private_key: String, // Base58 encoded private key or file path
    pub trade_amount_sol: f64,       // e.g. 0.04 SOL (~$6 to $8 depending on price)
    pub max_active_trades: usize,
    pub min_liquidity_usd: f64,      // e.g. 1000.0 (Trade if min liquidity is $1000)
    pub min_score_threshold: u32,   // Minimum token score (0-100) to execute trade
    pub max_slippage_percent: f64,   // e.g. 20.0 to 50.0%
    pub stop_loss_percent: f64,      // e.g. 40.0%
    pub take_profit_percent: f64,    // e.g. 200.0% (2x) to 500.0% (5x)
    pub priority_fee_lamports: u64,  // Priority fee to ensure ultra-fast execution
    pub enable_honeypot_checks: bool,
    pub max_bundled_supply_percent: f64, // e.g. 30.0% for top holders
    pub check_dev_history: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rpc_http_url: "https://api.mainnet-beta.solana.com".to_string(),
            rpc_ws_url: "wss://api.mainnet-beta.solana.com".to_string(),
            wallet_private_key: "YOUR_PRIVATE_KEY_BASE58_HERE".to_string(),
            trade_amount_sol: 0.04, // $6 Capital equivalent
            max_active_trades: 3,
            min_liquidity_usd: 1000.0,
            min_score_threshold: 65,
            max_slippage_percent: 25.0,
            stop_loss_percent: 40.0,
            take_profit_percent: 200.0, // 2x take-profit
            priority_fee_lamports: 100_000, // Speed up VM transactions
            enable_honeypot_checks: true,
            max_bundled_supply_percent: 30.0,
            check_dev_history: true,
        }
    }
}

pub fn load_config<P: AsRef<Path>>(path: P) -> Result<Config, Box<dyn std::error::Error>> {
    if !path.as_ref().exists() {
        let default_config = Config::default();
        let file = File::create(&path)?;
        serde_json::to_writer_pretty(file, &default_config)?;
        return Ok(default_config);
    }

    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let config: Config = serde_json::from_str(&contents)?;
    Ok(config)
}
