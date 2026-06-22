use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub name: String,
    pub symbol: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub showName: Option<bool>,
    pub createdOn: Option<String>,
    pub twitter: Option<String>,
    pub telegram: Option<String>,
    pub website: Option<String>,
}

pub struct TokenScorer {
    rpc_client: RpcClient,
    http_client: reqwest::Client,
    trending_keywords: HashSet<String>,
}

impl TokenScorer {
    pub fn new(rpc_client: RpcClient) -> Self {
        let mut trending = HashSet::new();
        // Popular meme and narrative keywords to scan tickers for
        for kw in &["doge", "shib", "pepe", "wif", "bonk", "trump", "elon", "musk", "ai", "sol", "pump", "giga", "chad", "kitty", "roaring"] {
            trending.insert(kw.to_string());
        }

        Self {
            rpc_client,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(1500)) // Low timeout to keep bot blazing fast
                .build()
                .unwrap_or_default(),
            trending_keywords: trending,
        }
    }

    /// Calculate Token Score (0 - 100)
    pub async fn score_token(
        &self,
        mint: &Pubkey,
        dev_wallet: &Pubkey,
        metadata_uri: &str,
    ) -> (u32, Vec<String>) {
        let mut score = 20; // Base score
        let mut reasons = Vec::new();

        // 1. Audit Token Name & Ticker Quality
        let mut has_trending_keyword = false;
        let mut has_gibberish_ticker = false;

        // Extract metadata info if we can fetch it fast
        let mut fetched_meta: Option<TokenMetadata> = None;
        if !metadata_uri.is_empty() {
            if let Ok(response) = self.http_client.get(metadata_uri).send().await {
                if let Ok(meta) = response.json::<TokenMetadata>().await {
                    fetched_meta = Some(meta);
                }
            }
        }

        if let Some(ref meta) = fetched_meta {
            let ticker_lower = meta.symbol.to_lowercase();
            let name_lower = meta.name.to_lowercase();

            // Check for trending keywords
            for kw in &self.trending_keywords {
                if ticker_lower.contains(kw) || name_lower.contains(kw) {
                    has_trending_keyword = true;
                    break;
                }
            }

            if has_trending_keyword {
                score += 15;
                reasons.push("Trending narrative keyword match (+15)".to_string());
            }

            // Check for gibberish tickers (e.g. "a1b2c3d4")
            if ticker_lower.len() > 6 && ticker_lower.chars().all(|c| c.is_alphanumeric()) && ticker_lower.chars().any(|c| c.is_numeric()) {
                has_gibberish_ticker = true;
                score = score.saturating_sub(15);
                reasons.push("Gibberish alphanumeric ticker detected (-15)".to_string());
            } else {
                score += 5;
                reasons.push("Clean ticker structure (+5)".to_string());
            }

            // 2. Validate Social Media Presence
            let mut social_count = 0;
            if meta.website.is_some() && !meta.website.as_ref().unwrap().is_empty() {
                social_count += 1;
            }
            if meta.twitter.is_some() && !meta.twitter.as_ref().unwrap().is_empty() {
                social_count += 1;
            }
            if meta.telegram.is_some() && !meta.telegram.as_ref().unwrap().is_empty() {
                social_count += 1;
            }

            match social_count {
                3 => {
                    score += 25;
                    reasons.push("Full Social Suite (Website, Twitter, Telegram) present (+25)".to_string());
                }
                2 => {
                    score += 15;
                    reasons.push("Double socials present (+15)".to_string());
                }
                1 => {
                    score += 8;
                    reasons.push("Single social link present (+8)".to_string());
                }
                _ => {
                    score = score.saturating_sub(10);
                    reasons.push("No socials provided — typical dev pump & dump profile (-10)".to_string());
                }
            }
        } else {
            reasons.push("Metadata fetch timed out or unavailable (neutral scoring)".to_string());
        }

        // 3. Dev Financial Commitment Check (Wallet Balance)
        if let Ok(balance) = self.rpc_client.get_balance(dev_wallet) {
            let balance_sol = balance as f64 / 1_000_000_000.0;
            if balance_sol > 1.0 {
                score += 20;
                reasons.push(format!("Dev wallet has substantial backing: {:.2} SOL (+20)", balance_sol));
            } else if balance_sol > 0.1 {
                score += 10;
                reasons.push(format!("Dev wallet has moderate backing: {:.2} SOL (+10)", balance_sol));
            } else {
                score = score.saturating_sub(15);
                reasons.push("Dev wallet is dust / empty (-15)".to_string());
            }
        }

        // 4. Initial Buy Patterns (Organic vs Bottled sniper launches)
        // Check if there are some initial purchases. Organic tokens have slow but consistent buys.
        if let Ok(signatures) = self.rpc_client.get_signatures_for_address_with_config(
            mint,
            solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config {
                before: None,
                until: None,
                limit: Some(10),
                commitment: None,
            }
        ) {
            let tx_count = signatures.len();
            if tx_count >= 5 {
                score += 10;
                reasons.push("Strong initial block buy volume (+10)".to_string());
            } else if tx_count == 1 {
                score += 5;
                reasons.push("Fresh launch, single developer buy (+5)".to_string());
            }
        }

        (score.min(100), reasons)
    }
}
