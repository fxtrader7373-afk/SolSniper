use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
    system_program,
};
use spl_associated_token_account::get_associated_token_address;
use spl_token::ID as TOKEN_PROGRAM_ID;
use rand::seq::SliceRandom;
use std::str::FromStr;
use std::time::Instant;

pub const PUMPFUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
pub const PUMPFUN_GLOBAL_ACCOUNT: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
pub const PUMPFUN_EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7Xx6SgqR";
pub const FEE_PROGRAM: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";

// 8 BREAKING_FEE_RECIPIENTS from April 2026 upgrade to disperse transaction congestion
pub const BREAKING_FEE_RECIPIENTS: [&str; 8] = [
    "7VtfL8fvgNfhz17qKRMjzQEXgbdpnHHHQRh54R9jP2RJ",
    "7hTckgnGnLQR6sdH7YkqFTAA7VwTfYFaZ6EhEsU3saCX",
    "9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz",
    "AVmoTthdrX6tKt4nDjco2D775W2YK3sDhxPcMmzUAmTY",
    "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM",
    "FWsW1xNtWscwNmKv6wVsU1iTzRN6wmmk3MjxRP5tT7hz",
    "G5UZAVbAf46s7cKWoyKu8kYTip9DGTpbLZ2qa9Aq69dP",
    "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV2fskvCwf8gCDbZ", // Fallback standard
];

// Discriminators for Pump.fun instructions
pub const BUY_DISCRIMINATOR: [u8; 8] = [0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea];
pub const SELL_DISCRIMINATOR: [u8; 8] = [0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad];

#[derive(Debug, Clone)]
pub struct TradePosition {
    pub mint: Pubkey,
    pub entry_price_sol: f64,
    pub token_balance: u64,
    pub purchase_time: Instant,
    pub bonding_curve: Pubkey,
    pub associated_bonding_curve: Pubkey,
}

pub struct TradeManager {
    rpc_client: RpcClient,
}

impl TradeManager {
    pub fn new(rpc_client: RpcClient) -> Self {
        Self { rpc_client }
    }

    /// Calculate the tokens to buy based on the constant product formula
    /// of the bonding curve. This avoids transaction failures.
    pub fn calculate_tokens_to_buy(
        &self,
        virtual_sol_reserves: u64,
        virtual_token_reserves: u64,
        buy_amount_lamports: u64,
    ) -> u64 {
        let new_sol_reserves = virtual_sol_reserves + buy_amount_lamports;
        // Constant Product: K = Sol * Token
        let k = (virtual_sol_reserves as u128) * (virtual_token_reserves as u128);
        let new_token_reserves = (k / new_sol_reserves as u128) as u64;
        virtual_token_reserves - new_token_reserves
    }

    /// Create and send a Pump.fun Buy transaction
    pub async fn buy_token(
        &self,
        wallet: &Keypair,
        mint: &Pubkey,
        creator: &Pubkey,
        amount_sol: f64,
        slippage_percent: f64,
        priority_fee_lamports: u64,
    ) -> Result<TradePosition, Box<dyn std::error::Error>> {
        let start = Instant::now();

        let program_id = Pubkey::from_str(PUMPFUN_PROGRAM)?;
        let global = Pubkey::from_str(PUMPFUN_GLOBAL_ACCOUNT)?;
        let event_authority = Pubkey::from_str(PUMPFUN_EVENT_AUTHORITY)?;
        let fee_program = Pubkey::from_str(FEE_PROGRAM)?;

        // Derive necessary PDAs
        let (bonding_curve, _) = Pubkey::find_program_address(
            &[b"bonding-curve", mint.as_ref()],
            &program_id,
        );
        let associated_bonding_curve = get_associated_token_address(&bonding_curve, mint);
        let user_token_account = get_associated_token_address(&wallet.pubkey(), mint);

        let (creator_vault, _) = Pubkey::find_program_address(
            &[b"creator-vault", creator.as_ref()],
            &program_id,
        );
        let (global_volume_accumulator, _) = Pubkey::find_program_address(
            &[b"global_volume_accumulator"],
            &program_id,
        );
        let (user_volume_accumulator, _) = Pubkey::find_program_address(
            &[b"user_volume_accumulator", wallet.pubkey().as_ref()],
            &program_id,
        );
        let (fee_config, _) = Pubkey::find_program_address(
            &[b"fee_config", program_id.as_ref()],
            &fee_program,
        );
        let (bonding_curve_v2, _) = Pubkey::find_program_address(
            &[b"bonding-curve-v2", mint.as_ref()],
            &program_id,
        );

        // Fetch Bonding Curve Reserves for accurate amount calculation
        let bonding_curve_data = self.rpc_client.get_account_data(&bonding_curve)?;
        if bonding_curve_data.len() < 40 {
            return Err("Invalid bonding curve account data".into());
        }
        // Read reserves (with 8-byte anchor discriminator offset)
        let virtual_token_reserves = u64::from_le_bytes(bonding_curve_data[8..16].try_into()?);
        let virtual_sol_reserves = u64::from_le_bytes(bonding_curve_data[16..24].try_into()?);

        let buy_amount_lamports = (amount_sol * 1_000_000_000.0) as u64;
        let tokens_to_buy = self.calculate_tokens_to_buy(
            virtual_sol_reserves,
            virtual_token_reserves,
            buy_amount_lamports,
        );

        // Slippage Calculation
        let max_sol_cost = (buy_amount_lamports as f64 * (1.0 + slippage_percent / 100.0)) as u64;

        // Build instructions
        let mut instructions = Vec::new();

        // 1. Compute Budget Instructions for ultra-fast transactions
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(100_000));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(priority_fee_lamports));

        // 2. Create Associated Token Account for user
        let create_ata_ix = spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &wallet.pubkey(),
            &wallet.pubkey(),
            mint,
            &TOKEN_PROGRAM_ID,
        );
        instructions.push(create_ata_ix);

        // 3. Build the Buy instruction data structure
        let mut buy_data = Vec::new();
        buy_data.extend_from_slice(&BUY_DISCRIMINATOR);
        buy_data.extend_from_slice(&tokens_to_buy.to_le_bytes());
        buy_data.extend_from_slice(&max_sol_cost.to_le_bytes());

        // Select a randomized fee recipient from the 8 BREAKING_FEE_RECIPIENTS to disperse congestion
        let random_fee_str = BREAKING_FEE_RECIPIENTS.choose(&mut rand::thread_rng()).unwrap();
        let fee_recipient = Pubkey::from_str(random_fee_str)?;

        // Order of accounts matching the 18 accounts from the latest April 2026 upgrade
        let accounts = vec![
            AccountMeta::new_readonly(global, false),                  // 0
            AccountMeta::new(fee_recipient, false),                     // 1
            AccountMeta::new_readonly(*mint, false),                    // 2
            AccountMeta::new(bonding_curve, false),                     // 3
            AccountMeta::new(associated_bonding_curve, false),          // 4
            AccountMeta::new(user_token_account, false),                // 5
            AccountMeta::new(wallet.pubkey(), true),                    // 6 (signer)
            AccountMeta::new_readonly(system_program::ID, false),       // 7
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),         // 8
            AccountMeta::new(creator_vault, false),                     // 9
            AccountMeta::new_readonly(event_authority, false),          // 10
            AccountMeta::new_readonly(program_id, false),               // 11
            AccountMeta::new(global_volume_accumulator, false),         // 12
            AccountMeta::new(user_volume_accumulator, false),          // 13
            AccountMeta::new_readonly(fee_config, false),               // 14
            AccountMeta::new_readonly(fee_program, false),              // 15
            AccountMeta::new_readonly(bonding_curve_v2, false),         // 16
            AccountMeta::new(fee_recipient, false),                     // 17 (trailing randomized fee)
        ];

        let buy_instruction = Instruction {
            program_id,
            accounts,
            data: buy_data,
        };
        instructions.push(buy_instruction);

        // Send and confirm transaction with high speed
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&wallet.pubkey()),
            &[wallet],
            recent_blockhash,
        );

        // Bypass preflight for the fastest possible land
        let config = solana_client::rpc_config::RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: None,
            encoding: None,
            max_retries: Some(3),
            min_context_slot: None,
        };

        let signature = self.rpc_client.send_transaction_with_config(&tx, config)?;
        
        // Wait for confirmation asynchronously
        let confirm_start = Instant::now();
        while confirm_start.elapsed().as_secs() < 10 {
            if self.rpc_client.confirm_transaction(&signature)? {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        }

        let entry_price_sol = amount_sol / (tokens_to_buy as f64 / 1_000_000.0);

        Ok(TradePosition {
            mint: *mint,
            entry_price_sol,
            token_balance: tokens_to_buy,
            purchase_time: Instant::now(),
            bonding_curve,
            associated_bonding_curve,
        })
    }

    /// Create and send a Pump.fun Sell transaction
    pub async fn sell_token(
        &self,
        wallet: &Keypair,
        position: &TradePosition,
        slippage_percent: f64,
        priority_fee_lamports: u64,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let program_id = Pubkey::from_str(PUMPFUN_PROGRAM)?;
        let global = Pubkey::from_str(PUMPFUN_GLOBAL_ACCOUNT)?;
        let event_authority = Pubkey::from_str(PUMPFUN_EVENT_AUTHORITY)?;
        let fee_program = Pubkey::from_str(FEE_PROGRAM)?;

        let user_token_account = get_associated_token_address(&wallet.pubkey(), &position.mint);

        let (global_volume_accumulator, _) = Pubkey::find_program_address(
            &[b"global_volume_accumulator"],
            &program_id,
        );
        let (user_volume_accumulator, _) = Pubkey::find_program_address(
            &[b"user_volume_accumulator", wallet.pubkey().as_ref()],
            &program_id,
        );
        let (fee_config, _) = Pubkey::find_program_address(
            &[b"fee_config", program_id.as_ref()],
            &fee_program,
        );
        let (bonding_curve_v2, _) = Pubkey::find_program_address(
            &[b"bonding-curve-v2", position.mint.as_ref()],
            &program_id,
        );

        // Fetch Bonding Curve Reserves for accurate sell calculation
        let bonding_curve_data = self.rpc_client.get_account_data(&position.bonding_curve)?;
        let virtual_token_reserves = u64::from_le_bytes(bonding_curve_data[8..16].try_into()?);
        let virtual_sol_reserves = u64::from_le_bytes(bonding_curve_data[16..24].try_into()?);

        // Calculate expected output lamports: K = Sol * Token
        // virtual_sol_reserves * virtual_token_reserves = (virtual_sol_reserves - out_sol) * (virtual_token_reserves + in_tokens)
        let new_token_reserves = virtual_token_reserves + position.token_balance;
        let k = (virtual_sol_reserves as u128) * (virtual_token_reserves as u128);
        let new_sol_reserves = (k / new_token_reserves as u128) as u64;
        let expected_sol_out = virtual_sol_reserves - new_sol_reserves;

        let min_sol_out = (expected_sol_out as f64 * (1.0 - slippage_percent / 100.0)) as u64;

        let mut instructions = Vec::new();
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(100_000));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(priority_fee_lamports));

        let mut sell_data = Vec::new();
        sell_data.extend_from_slice(&SELL_DISCRIMINATOR);
        sell_data.extend_from_slice(&position.token_balance.to_le_bytes());
        sell_data.extend_from_slice(&min_sol_out.to_le_bytes());

        let random_fee_str = BREAKING_FEE_RECIPIENTS.choose(&mut rand::thread_rng()).unwrap();
        let fee_recipient = Pubkey::from_str(random_fee_str)?;

        // 16 Accounts layout on Sell
        let accounts = vec![
            AccountMeta::new_readonly(global, false),                  // 0
            AccountMeta::new(fee_recipient, false),                     // 1
            AccountMeta::new_readonly(position.mint, false),            // 2
            AccountMeta::new(position.bonding_curve, false),            // 3
            AccountMeta::new(position.associated_bonding_curve, false), // 4
            AccountMeta::new(user_token_account, false),                // 5
            AccountMeta::new(wallet.pubkey(), true),                    // 6 (signer)
            AccountMeta::new_readonly(system_program::ID, false),       // 7
            AccountMeta::new_readonly(spl_associated_token_account::ID, false), // 8 (associated token program)
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),         // 9
            AccountMeta::new_readonly(event_authority, false),          // 10
            AccountMeta::new_readonly(program_id, false),               // 11
            AccountMeta::new(global_volume_accumulator, false),         // 12
            AccountMeta::new(user_volume_accumulator, false),          // 13
            AccountMeta::new_readonly(fee_config, false),               // 14
            AccountMeta::new_readonly(bonding_curve_v2, false),         // 15
        ];

        let sell_instruction = Instruction {
            program_id,
            accounts,
            data: sell_data,
        };
        instructions.push(sell_instruction);

        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&wallet.pubkey()),
            &[wallet],
            recent_blockhash,
        );

        let config = solana_client::rpc_config::RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: None,
            encoding: None,
            max_retries: Some(3),
            min_context_slot: None,
        };

        let signature = self.rpc_client.send_transaction_with_config(&tx, config)?;
        
        let confirm_start = Instant::now();
        while confirm_start.elapsed().as_secs() < 10 {
            if self.rpc_client.confirm_transaction(&signature)? {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        }

        Ok(expected_sol_out)
    }
}
