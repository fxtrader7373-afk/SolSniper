mod config;
mod solana;
mod honeypot;
mod scorer;
mod trade;
mod dashboard;

use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use crate::config::{load_config, Config};
use crate::solana::{add_log, NewTokenEvent, SolanaListener};
use crate::honeypot::HoneypotChecker;
use crate::scorer::TokenScorer;
use crate::trade::{TradeManager, TradePosition};
use crate::dashboard::{DashboardCommand, DashboardStats, TerminalDashboard};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize Paths & Load Config
    let config_path = "config.json";
    let config = match load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}. Initializing default config...", e);
            Config::default()
        }
    };

    // 2. Decode Wallet Keypair
    let wallet = if config.wallet_private_key == "YOUR_PRIVATE_KEY_BASE58_HERE" || config.wallet_private_key.is_empty() {
        // Generating random wallet for dry runs to prevent crashes
        let kp = Keypair::new();
        println!("⚠️ WARNING: Using mock/newly-generated keypair since config contains default key. Pubkey: {}", kp.pubkey());
        kp
    } else {
        match Keypair::from_base58_string(&config.wallet_private_key) {
            Ok(k) => k,
            Err(_) => {
                let kp = Keypair::new();
                println!("⚠️ WARNING: Invalid base58 private key, generating random wallet: {}", kp.pubkey());
                kp
            }
        }
    };

    // 3. Connect to Solana Clients
    let rpc_client = RpcClient::new(config.rpc_http_url.clone());
    
    // Check connection & initial balance
    let balance_sol = match rpc_client.get_balance(&wallet.pubkey()) {
        Ok(bal) => bal as f64 / 1_000_000_000.0,
        Err(_) => {
            println!("Could not fetch balance. Using simulated 1.5 SOL for display.");
            1.5
        }
    };

    // 4. Set up shared Dashboard State
    let stats = Arc::new(Mutex::new(DashboardStats {
        status: "RUNNING".to_string(),
        balance_sol,
        total_scanned: 0,
        total_trades: 0,
        wins: 0,
        losses: 0,
        win_rate: 0.0,
        net_pnl_sol: 0.0,
        active_positions: 0,
        current_capital_sol: config.trade_amount_sol,
        rpc_latency_ms: 22, // default baseline
        sniper_mode: "AUTO".to_string(),
        min_score_threshold: config.min_score_threshold,
        max_slippage_percent: config.max_slippage_percent,
        last_sniped_token: "".to_string(),
    }));

    let logs = Arc::new(Mutex::new(Vec::new()));
    add_log(&logs, "Pump.fun Sniper Bot Initialized Successfully!");
    add_log(&logs, &format!("Wallet loaded: {}", wallet.pubkey().to_string().chars().take(8).collect::<String>() + "..."));

    // 5. Setup Communication Channels
    let (event_tx, mut event_rx) = mpsc::channel::<NewTokenEvent>(100);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<DashboardCommand>(10);

    // 6. Spawn Background Solana Listener Thread
    let listener = SolanaListener::new(
        config.rpc_http_url.clone(),
        config.rpc_ws_url.clone(),
        event_tx,
        logs.clone(),
    );
    listener.start_listening().await;

    // 7. Instantiate Scorer, Honeypot and Trade Manager
    let scorer = Arc::new(TokenScorer::new(RpcClient::new(config.rpc_http_url.clone())));
    let honeypot = Arc::new(HoneypotChecker::new(RpcClient::new(config.rpc_http_url.clone())));
    let trade_manager = Arc::new(TradeManager::new(RpcClient::new(config.rpc_http_url.clone())));
    
    let active_positions: Arc<Mutex<HashMap<Pubkey, TradePosition>>> = Arc::new(Mutex::new(HashMap::new()));
    let wallet_arc = Arc::new(wallet);

    // 8. Spawn Pipeline Engine Loop (Handles Sniping Events)
    let stats_pipeline = stats.clone();
    let logs_pipeline = logs.clone();
    let honeypot_pipeline = honeypot.clone();
    let scorer_pipeline = scorer.clone();
    let trade_manager_pipeline = trade_manager.clone();
    let active_positions_pipeline = active_positions.clone();
    let wallet_pipeline = wallet_arc.clone();
    let config_pipeline = config.clone();

    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let mut current_stats = stats_pipeline.lock().unwrap().clone();
            if current_stats.status != "RUNNING" {
                continue;
            }

            // Increment Scanned Tokens
            {
                let mut s = stats_pipeline.lock().unwrap();
                s.total_scanned += 1;
            }

            // A. Honeypot check
            if config_pipeline.enable_honeypot_checks {
                match honeypot_pipeline.check_token(&event.mint, &event.creator, config_pipeline.max_bundled_supply_percent).await {
                    Ok(report) => {
                        if report.is_honeypot {
                            add_log(&logs_pipeline, &format!("⚠️ SKIPPED: Token {} flagged as HONEY TRAP (Risk: {}/100)", event.mint.to_string().chars().take(6).collect::<String>(), report.risk_score));
                            for reason in report.warning_reasons {
                                add_log(&logs_pipeline, &format!("   └─ Reason: {}", reason));
                            }
                            continue;
                        }
                    }
                    Err(_) => {
                        add_log(&logs_pipeline, "Honeypot check failed (RPC Timeout). Proceeding with extreme caution.");
                    }
                }
            }

            // B. Scoring Check ("Advanced Mind" analysis)
            let (score, score_reasons) = scorer_pipeline.score_token(&event.mint, &event.creator, &event.metadata_uri).await;
            
            if score < current_stats.min_score_threshold {
                add_log(&logs_pipeline, &format!("⚙️ SKIPPED: Token {} got Score: {} (Threshold: {})", event.mint.to_string().chars().take(6).collect::<String>(), score, current_stats.min_score_threshold));
                continue;
            }

            // Log details of high scoring token
            add_log(&logs_pipeline, &format!("🔥 SMART RATING: Token {} rated {}/100!", event.mint.to_string().chars().take(8).collect::<String>(), score));
            for reason in score_reasons {
                add_log(&logs_pipeline, &format!("   ├─ {}", reason));
            }

            // Check if we already own this token or reached position limits
            let positions_len = active_positions_pipeline.lock().unwrap().len();
            if positions_len >= config_pipeline.max_active_trades {
                add_log(&logs_pipeline, "⚠️ SKIPPED: Max active positions limit reached.");
                continue;
            }

            // C. Execute Buy Snipe ($6 Capital)
            if current_stats.sniper_mode == "AUTO" {
                add_log(&logs_pipeline, &format!("🚀 SNIPING: Buying token {} with {:.4} SOL...", event.mint.to_string().chars().take(6).collect::<String>(), current_stats.current_capital_sol));
                
                match trade_manager_pipeline.buy_token(
                    &wallet_pipeline,
                    &event.mint,
                    &event.creator,
                    current_stats.current_capital_sol,
                    current_stats.max_slippage_percent,
                    config_pipeline.priority_fee_lamports,
                ).await {
                    Ok(position) => {
                        add_log(&logs_pipeline, &format!("✅ BOUGHT SUCCESS: Entry price {:.8} SOL per token.", position.entry_price_sol));
                        active_positions_pipeline.lock().unwrap().insert(event.mint, position);
                        
                        let mut s = stats_pipeline.lock().unwrap();
                        s.total_trades += 1;
                        s.active_positions = active_positions_pipeline.lock().unwrap().len();
                        s.last_sniped_token = event.mint.to_string().chars().take(8).collect();
                    }
                    Err(e) => {
                        add_log(&logs_pipeline, &format!("❌ BUY FAILED for {}: {:?}", event.mint.to_string().chars().take(6).collect::<String>(), e));
                    }
                }
            }
        }
    });

    // 9. Spawn Positions Manager Loop (Stop Loss / Take Profit Tracker)
    let stats_manager = stats.clone();
    let logs_manager = logs.clone();
    let active_positions_manager = active_positions.clone();
    let trade_manager_pos = trade_manager.clone();
    let wallet_manager = wallet_arc.clone();
    let config_manager = config.clone();

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await; // Poll prices every 1.5s
            
            let mut positions_to_sell = Vec::new();
            {
                let positions = active_positions_manager.lock().unwrap();
                for (mint, pos) in positions.iter() {
                    // Fetch current pricing from bonding curve reserves
                    let program_id = Pubkey::from_str(crate::trade::PUMPFUN_PROGRAM).unwrap();
                    let (bonding_curve, _) = Pubkey::find_program_address(
                        &[b"bonding-curve", mint.as_ref()],
                        &program_id,
                    );
                    
                    if let Ok(bonding_curve_data) = rpc_client.get_account_data(&bonding_curve) {
                        if bonding_curve_data.len() >= 24 {
                            let virtual_token_reserves = u64::from_le_bytes(bonding_curve_data[8..16].try_into().unwrap()) as f64;
                            let virtual_sol_reserves = u64::from_le_bytes(bonding_curve_data[16..24].try_into().unwrap()) as f64;
                            
                            let current_price_sol = virtual_sol_reserves / virtual_token_reserves;
                            let change_percent = ((current_price_sol - pos.entry_price_sol) / pos.entry_price_sol) * 100.0;
                            
                            // Check Stop Loss (-40%) or Take Profit (2x = +100%, up to 5x = +400%)
                            if change_percent <= -config_manager.stop_loss_percent {
                                positions_to_sell.push((*mint, "Stop-Loss Hit (-40%)".to_string(), change_percent));
                            } else if change_percent >= config_manager.take_profit_percent {
                                positions_to_sell.push((*mint, "Take-Profit Triggered (2x+)".to_string(), change_percent));
                            }
                        }
                    }
                }
            }

            // Execute Sells
            for (mint, reason, pnl_pct) in positions_to_sell {
                let position_to_sell = {
                    let mut pos_map = active_positions_manager.lock().unwrap();
                    pos_map.remove(&mint)
                };

                if let Some(pos) = position_to_sell {
                    add_log(&logs_manager, &format!("⚠️ SELLING: {} on {} (PnL: {:.1}%)", pos.mint.to_string().chars().take(6).collect::<String>(), reason, pnl_pct));
                    
                    match trade_manager_pos.sell_token(&wallet_manager, &pos, config_manager.max_slippage_percent, config_manager.priority_fee_lamports).await {
                        Ok(sol_received) => {
                            let sol_spent = (pos.entry_price_sol * (pos.token_balance as f64 / 1_000_000.0));
                            let trade_pnl = (sol_received as f64 / 1_000_000_000.0) - sol_spent;
                            
                            add_log(&logs_manager, &format!("💰 SOLD SUCCESS: Realized PnL: {:.4} SOL!", trade_pnl));
                            
                            let mut s = stats_manager.lock().unwrap();
                            s.net_pnl_sol += trade_pnl;
                            if trade_pnl > 0.0 {
                                s.wins += 1;
                            } else {
                                s.losses += 1;
                            }
                            let total = s.wins + s.losses;
                            if total > 0 {
                                s.win_rate = (s.wins as f64 / total as f64) * 100.0;
                            }
                            s.active_positions = active_positions_manager.lock().unwrap().len();
                        }
                        Err(e) => {
                            add_log(&logs_manager, &format!("❌ SELL FAILED: {:?}", e));
                            // Re-insert position to retry next loop
                            active_positions_manager.lock().unwrap().insert(mint, pos);
                        }
                    }
                }
            }
        }
    });

    // 10. Start Command Listener Loop (Interprets TUI controls)
    let stats_cmd = stats.clone();
    let logs_cmd = logs.clone();
    let active_positions_cmd = active_positions.clone();
    let trade_manager_cmd = trade_manager.clone();
    let wallet_cmd = wallet_arc.clone();
    let config_cmd = config.clone();

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                DashboardCommand::ToggleStatus => {
                    let mut s = stats_cmd.lock().unwrap();
                    if s.status == "RUNNING" {
                        s.status = "PAUSED".to_string();
                        add_log(&logs_cmd, "⏸️ Sniper bot PAUSED by user.");
                    } else {
                        s.status = "RUNNING".to_string();
                        add_log(&logs_cmd, "▶️ Sniper bot RESUMED by user.");
                    }
                }
                DashboardCommand::ToggleMode => {
                    let mut s = stats_cmd.lock().unwrap();
                    if s.sniper_mode == "AUTO" {
                        s.sniper_mode = "MANUAL".to_string();
                        add_log(&logs_cmd, "🔧 Sniper mode switched to MANUAL.");
                    } else {
                        s.sniper_mode = "AUTO".to_string();
                        add_log(&logs_cmd, "🤖 Sniper mode switched to AUTO.");
                    }
                }
                DashboardCommand::EmergencySellAll => {
                    add_log(&logs_cmd, "🚨 EMERGENCY SELL ALL TRIGGERED!");
                    let mut positions = active_positions_cmd.lock().unwrap();
                    let mints: Vec<Pubkey> = positions.keys().cloned().collect();
                    
                    for mint in mints {
                        if let Some(pos) = positions.remove(&mint) {
                            add_log(&logs_cmd, &format!("🚨 Emergency Selling {}...", pos.mint));
                            let tm = trade_manager_cmd.clone();
                            let w = wallet_cmd.clone();
                            let conf = config_cmd.clone();
                            tokio::spawn(async move {
                                let _ = tm.sell_token(&w, &pos, conf.max_slippage_percent, conf.priority_fee_lamports).await;
                            });
                        }
                    }
                    stats_cmd.lock().unwrap().active_positions = 0;
                }
                DashboardCommand::IncreaseCapital => {
                    let mut s = stats_cmd.lock().unwrap();
                    s.current_capital_sol += 0.01;
                    add_log(&logs_cmd, &format!("📈 Buy size increased to: {:.4} SOL", s.current_capital_sol));
                }
                DashboardCommand::DecreaseCapital => {
                    let mut s = stats_cmd.lock().unwrap();
                    if s.current_capital_sol > 0.01 {
                        s.current_capital_sol -= 0.01;
                        add_log(&logs_cmd, &format!("📉 Buy size decreased to: {:.4} SOL", s.current_capital_sol));
                    }
                }
                DashboardCommand::IncreaseScore => {
                    let mut s = stats_cmd.lock().unwrap();
                    if s.min_score_threshold <= 95 {
                        s.min_score_threshold += 5;
                        add_log(&logs_cmd, &format!("⚙️ Minimum rating threshold increased to: {}", s.min_score_threshold));
                    }
                }
                DashboardCommand::DecreaseScore => {
                    let mut s = stats_cmd.lock().unwrap();
                    if s.min_score_threshold >= 5 {
                        s.min_score_threshold -= 5;
                        add_log(&logs_cmd, &format!("⚙️ Minimum rating threshold decreased to: {}", s.min_score_threshold));
                    }
                }
                DashboardCommand::Quit => {
                    add_log(&logs_cmd, "Shutting down safely...");
                }
            }
        }
    });

    // 11. Run Terminal TUI (Interactive Control Panel Screen)
    TerminalDashboard::setup_terminal()?;
    let dashboard = TerminalDashboard::new(stats.clone(), logs.clone(), cmd_tx);
    
    // Will block until the user presses 'Q'
    let run_res = dashboard.run().await;
    
    TerminalDashboard::restore_terminal()?;
    
    if let Err(e) = run_res {
        eprintln!("Dashboard execution error: {:?}", e);
    }

    println!("👋 Pump.fun Sniper Bot stopped safely. Have a great day!");
    Ok(())
}
