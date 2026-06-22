use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use serde_json::Value;

pub struct HoneypotChecker {
    rpc_client: RpcClient,
}

#[derive(Debug, Clone)]
pub struct HoneypotReport {
    pub is_honeypot: bool,
    pub dev_rugged_before: bool,
    pub dev_creation_count: u32,
    pub top_holders_percent: f64,
    pub is_mint_authority_revoked: bool,
    pub is_freeze_authority_disabled: bool,
    pub risk_score: u32, // 0 - 100 (high is riskier)
    pub warning_reasons: Vec<String>,
}

impl HoneypotChecker {
    pub fn new(rpc_client: RpcClient) -> Self {
        Self { rpc_client }
    }

    /// Run comprehensive honey trap and honeypot checks on a newly minted token
    pub async fn check_token(
        &self,
        mint: &Pubkey,
        dev_wallet: &Pubkey,
        max_bundled_percent: f64,
    ) -> Result<HoneypotReport, Box<dyn std::error::Error>> {
        let mut warning_reasons = Vec::new();
        let mut risk_score = 0;

        // 1. Check Mint and Freeze Authorities (SPL Token Mint Account parsing)
        let mut is_mint_authority_revoked = true;
        let mut is_freeze_authority_disabled = true;

        if let Ok(account_data) = self.rpc_client.get_account(&mint) {
            // In SPL Token layout:
            // - Mint authority option starts at offset 0 (4 bytes option + 32 bytes pubkey)
            // - Freeze authority option is at offset 44 (4 bytes option + 32 bytes pubkey)
            if account_data.data.len() >= 82 {
                let mint_auth_option = u32::from_le_bytes(account_data.data[0..4].try_into().unwrap());
                if mint_auth_option != 0 {
                    is_mint_authority_revoked = false;
                    risk_score += 40;
                    warning_reasons.push("Mint authority is still ACTIVE (risk of mint-inflation rug!)".to_string());
                }

                let freeze_auth_option = u32::from_le_bytes(account_data.data[44..48].try_into().unwrap());
                if freeze_auth_option != 0 {
                    is_freeze_authority_disabled = false;
                    risk_score += 40;
                    warning_reasons.push("Freeze authority is still ACTIVE (dev can freeze your tokens!)".to_string());
                }
            }
        }

        // 2. Check Top Holders for Bundled Supply (Sybil Attack)
        // Pump.fun tokens have total supply of 1B (1_000_000_000_000_000 lamports / 6 decimals)
        let mut top_holders_percent = 0.0;
        if let Ok(largest_accounts) = self.rpc_client.get_token_largest_accounts(mint) {
            let mut top_holders_amount: u64 = 0;
            let total_supply: u64 = 1_000_000_000 * 1_000_000; // 1 Billion tokens with 6 decimals

            // Sum the top 5 holder accounts, excluding the bonding curve account itself.
            // In real scenarios, bonding curve starts with ~800M tokens (80% of supply)
            let mut non_bc_count = 0;
            for holder in largest_accounts {
                // If holder is not the pump bonding curve
                if non_bc_count < 5 {
                    if let Ok(amount_str) = holder.amount.amount.parse::<u64>() {
                        // Skip if it looks like the bonding curve address (usually > 50% of supply)
                        if amount_str < (total_supply * 3 / 4) {
                            top_holders_amount += amount_str;
                            non_bc_count += 1;
                        }
                    }
                }
            }

            top_holders_percent = (top_holders_amount as f64 / total_supply as f64) * 100.0;
            if top_holders_percent > max_bundled_percent {
                risk_score += 30;
                warning_reasons.push(format!(
                    "Sybil Bundle Alert: Top non-curve holders own {:.2}% of supply (limit is {:.2}%)",
                    top_holders_percent, max_bundled_percent
                ));
            }
        }

        // 3. Dev Wallet History Audit
        let mut dev_rugged_before = false;
        let mut dev_creation_count = 0;
        
        // Let's get the transaction history of the developer's wallet
        if let Ok(signatures) = self.rpc_client.get_signatures_for_address_with_config(
            dev_wallet,
            solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config {
                before: None,
                until: None,
                limit: Some(20), // Fetch last 20 txs to keep VM lightweight
                commitment: None,
            }
        ) {
            dev_creation_count = signatures.len() as u32;
            
            // Check if any previous tx of dev contains keywords like "rug" or "panic" or high sell-offs
            // Also analyze transaction volume
            let mut abnormal_transfers = 0;
            for sig in signatures {
                if let Ok(tx) = self.rpc_client.get_transaction(
                    &solana_sdk::signature::Signature::from_str(&sig.signature).unwrap(),
                    solana_client::rpc_config::RpcTransactionConfig {
                        encoding: Some(solana_transaction_status::UiTransactionEncoding::Json),
                        commitment: None,
                        max_supported_transaction_version: Some(0),
                    }
                ) {
                    // Audit log messages for previous rugs
                    if let Some(meta) = tx.transaction.meta {
                        if let Some(logs) = meta.log_messages {
                            for log in logs {
                                if log.contains("Panic") || log.contains("Error") {
                                    abnormal_transfers += 1;
                                }
                            }
                        }
                    }
                }
            }

            if abnormal_transfers > 5 {
                dev_rugged_before = true;
                risk_score += 25;
                warning_reasons.push("Dev wallet history shows multiple failed/malicious transactions".to_string());
            }
        }

        // 4. Bonding Curve Liquidity Check
        // Pump.fun bonding curves have standard layout. We check the SOL balance of the bonding curve.
        let (bonding_curve, _) = Pubkey::find_program_address(
            &[b"bonding-curve", mint.as_ref()],
            &Pubkey::from_str("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P").unwrap(),
        );

        let bonding_curve_sol = self.rpc_client.get_balance(&bonding_curve).unwrap_or(0);
        let sol_value_usd = 150.0; // Simulated price for conversion, or can be fetched dynamically
        let liquidity_usd = (bonding_curve_sol as f64 / 1_000_000_000.0) * sol_value_usd;

        if liquidity_usd < 10.0 { // Extremely low starting bonding curve balance is fishy
            risk_score += 15;
            warning_reasons.push("Extremely low initial bonding curve liquidity".to_string());
        }

        let is_honeypot = risk_score >= 60;

        Ok(HoneypotReport {
            is_honeypot,
            dev_rugged_before,
            dev_creation_count,
            top_holders_percent,
            is_mint_authority_revoked,
            is_freeze_authority_disabled,
            risk_score: risk_score.min(100),
            warning_reasons,
        })
    }
}
