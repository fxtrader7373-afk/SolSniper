use solana_client::{
    pubsub_client::PubsubClient,
    rpc_client::RpcClient,
};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_transaction_status::{UiTransactionEncoding, UiMessage};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use crate::trade::PUMPFUN_PROGRAM;

pub const CREATE_DISCRIMINATOR: [u8; 8] = [24, 30, 200, 40, 5, 28, 7, 119];

#[derive(Debug, Clone)]
pub struct NewTokenEvent {
    pub mint: Pubkey,
    pub creator: Pubkey,
    pub metadata_uri: String,
    pub timestamp: Instant,
}

pub struct SolanaListener {
    rpc_http_url: String,
    rpc_ws_url: String,
    event_tx: mpsc::Sender<NewTokenEvent>,
    logs: Arc<Mutex<Vec<String>>>,
}

impl SolanaListener {
    pub fn new(
        rpc_http_url: String,
        rpc_ws_url: String,
        event_tx: mpsc::Sender<NewTokenEvent>,
        logs: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            rpc_http_url,
            rpc_ws_url,
            event_tx,
            logs,
        }
    }

    /// Start listening to Pump.fun transaction logs over WebSocket
    pub async fn start_listening(self) {
        let rpc_ws_url_clone = self.rpc_ws_url.clone();
        let logs_clone = self.logs.clone();
        
        // Spawn asynchronous logging task
        tokio::spawn(async move {
            add_log(&logs_clone, "Initializing WebSocket Log Subscription...");
            
            // Loop for auto-reconnection of websocket stream
            loop {
                match PubsubClient::logs_subscribe(
                    &rpc_ws_url_clone,
                    solana_client::pubsub_client::PubsubClientFilter::Mentions(vec![
                        PUMPFUN_PROGRAM.to_string(),
                    ]),
                    solana_client::pubsub_client::PubsubClientConfig {
                        commitment: Some(solana_sdk::commitment_config::CommitmentConfig::confirmed()),
                    },
                ) {
                    Ok((_subscription, receiver)) => {
                        add_log(&logs_clone, "WS Connected to Solana Node - Listening for New Tokens...");
                        
                        while let Ok(log_response) = receiver.recv() {
                            let logs = log_response.value.logs;
                            
                            // Check if log contains "Instruction: Create"
                            let contains_create = logs.iter().any(|log| log.contains("Instruction: Create"));
                            if contains_create {
                                let signature_str = log_response.value.signature;
                                if let Ok(sig) = Signature::from_str(&signature_str) {
                                    let rpc_url = self.rpc_http_url.clone();
                                    let event_tx_inner = self.event_tx.clone();
                                    let logs_inner = self.logs.clone();
                                    
                                    // Process transaction in background thread immediately to maintain high speed
                                    tokio::spawn(async move {
                                        if let Err(e) = process_launch_tx(&rpc_url, sig, event_tx_inner, &logs_inner).await {
                                            // Silent debug or lightweight log
                                        }
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        add_log(&logs_clone, &format!("WS Subscription error: {:?}. Retrying in 3s...", e));
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    }
                }
            }
        });
    }
}

/// Parse the launch transaction details to extract mint, creator and metadata URI
async fn process_launch_tx(
    rpc_http_url: &str,
    signature: Signature,
    event_tx: mpsc::Sender<NewTokenEvent>,
    logs: &Arc<Mutex<Vec<String>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rpc_client = RpcClient::new(rpc_http_url.to_string());
    
    // Fetch transaction details (JSON format)
    let tx_detail = rpc_client.get_transaction_with_config(
        &signature,
        solana_client::rpc_config::RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::Json),
            commitment: Some(solana_sdk::commitment_config::CommitmentConfig::confirmed()),
            max_supported_transaction_version: Some(0),
        },
    )?;

    let meta = tx_detail.transaction.meta.ok_or("No transaction metadata found")?;
    if meta.err.is_some() {
        return Err("Transaction failed".into());
    }

    let ui_tx = tx_detail.transaction.transaction;
    
    // Extract accounts list and instructions
    if let UiMessage::Raw(raw_msg) = ui_tx.message {
        let account_keys: Vec<Pubkey> = raw_msg
            .account_keys
            .iter()
            .map(|key_str| Pubkey::from_str(key_str).unwrap_or_default())
            .collect();

        // Loop through all instructions to find the Pump.fun create instruction
        for ix in raw_msg.instructions {
            let program_id = account_keys[ix.program_id_index as usize];
            if program_id.to_string() == PUMPFUN_PROGRAM {
                // Decode instruction data (Base58 encoded in standard JSON Rpc responses)
                if let Ok(data_bytes) = bs58::decode(&ix.data).into_vec() {
                    if data_bytes.len() >= 8 && data_bytes[0..8] == CREATE_DISCRIMINATOR {
                        // Account Layout for 'create':
                        // Index 0: Mint
                        // Index 1: Bonding Curve
                        // Index 2: Associated Bonding Curve
                        // Index 3: Global
                        // Index 4: System Program
                        // Index 5: SPL Token Program
                        // Index 6: User (Creator)
                        if ix.accounts.len() >= 7 {
                            let mint_idx = ix.accounts[0] as usize;
                            let creator_idx = ix.accounts[6] as usize;
                            
                            let mint = account_keys[mint_idx];
                            let creator = account_keys[creator_idx];

                            // Decode metadata_uri string from Anchor instruction data
                            // Offset 8 onwards is: name (string), symbol (string), uri (string)
                            // Anchor strings have 4-byte length prefix
                            let mut offset = 8;
                            
                            // 1. Skip Name
                            if data_bytes.len() > offset + 4 {
                                let name_len = u32::from_le_bytes(data_bytes[offset..offset+4].try_into().unwrap()) as usize;
                                offset += 4 + name_len;
                            }
                            // 2. Skip Symbol
                            if data_bytes.len() > offset + 4 {
                                let symbol_len = u32::from_le_bytes(data_bytes[offset..offset+4].try_into().unwrap()) as usize;
                                offset += 4 + symbol_len;
                            }
                            // 3. Read Metadata URI
                            let mut metadata_uri = String::new();
                            if data_bytes.len() > offset + 4 {
                                let uri_len = u32::from_le_bytes(data_bytes[offset..offset+4].try_into().unwrap()) as usize;
                                if uri_len > 0 && uri_len < 2048 && data_bytes.len() >= offset + 4 + uri_len {
                                    let uri_bytes = &data_bytes[offset+4..offset+4+uri_len];
                                    if let Ok(uri_str) = String::from_utf8(uri_bytes.to_vec()) {
                                        metadata_uri = uri_str;
                                    }
                                }
                            }

                            // If metadata_uri is empty, fallback to pump.fun standard CDN
                            if metadata_uri.is_empty() {
                                metadata_uri = format!("https://ipfs.io/ipfs/Qm{}", mint);
                            }

                            add_log(logs, &format!("🎯 NEW PUMP TOKEN DETECTED: {}", mint.to_string().chars().take(8).collect::<String>() + "..."));

                            let event = NewTokenEvent {
                                mint,
                                creator,
                                metadata_uri,
                                timestamp: Instant::now(),
                            };

                            let _ = event_tx.send(event).await;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn add_log(logs: &Arc<Mutex<Vec<String>>>, message: &str) {
    let mut l = logs.lock().unwrap();
    let time = chrono::Local::now().format("%H:%M:%S");
    l.push(format!("[{}] {}", time, message));
    if l.len() > 100 {
        l.remove(0); // Cap history to prevent memory leaks on 1GB RAM VM
    }
}
