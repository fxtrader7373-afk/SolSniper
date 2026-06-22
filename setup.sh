#!/bin/bash

# ==============================================================================
#           ⚡ PUMPFUN ADVANCED SOLANA SNIPER BOT - SETUP SCRIPT ⚡
#           Optimized for Oracle Cloud VM (1 OCPU, 1GB RAM, Ubuntu)
# ==============================================================================

set -e # Exit immediately if a command exits with a non-zero status

COLOR_CYAN='\033[0;36m'
COLOR_GREEN='\033[0;32m'
COLOR_YELLOW='\033[1;33m'
COLOR_RED='\033[0;31m'
COLOR_RESET='\033[0m'

echo -e "${COLOR_CYAN}======================================================================${COLOR_RESET}"
echo -e "${COLOR_GREEN}      🚀 STARTING PUMPFUN SOLANA SNIPER BOT SETUP FOR ORACLE VM 🚀      ${COLOR_RESET}"
echo -e "${COLOR_CYAN}======================================================================${COLOR_RESET}"

# ------------------------------------------------------------------------------
# STEP 1: CONFIGURE SWAP SPACE (CRITICAL FOR 1GB RAM VM COMPILATION!)
# ------------------------------------------------------------------------------
echo -e "\n${COLOR_YELLOW}[Step 1/5] Configuring Swap Space to prevent Out-Of-Memory (OOM) compiler crashes...${COLOR_RESET}"
if [ -f /swapfile ]; then
    echo -e "${COLOR_GREEN}✔ Swap space already exists on this system.${COLOR_RESET}"
else
    echo -e "${COLOR_CYAN}Creating 2GB swap file... This will give your 1GB RAM VM 3GB of total virtual memory.${COLOR_RESET}"
    sudo fallocate -l 2G /swapfile || sudo dd if=/dev/zero of=/swapfile bs=1M count=2048
    sudo chmod 600 /swapfile
    sudo mkswap /swapfile
    sudo swapon /swapfile
    # Persist swap across reboots
    echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
    echo -e "${COLOR_GREEN}✔ 2GB Swap space successfully created and enabled!${COLOR_RESET}"
fi

# ------------------------------------------------------------------------------
# STEP 2: UPDATE REPOS AND INSTALL DEBIAN UTILITIES
# ------------------------------------------------------------------------------
echo -e "\n${COLOR_YELLOW}[Step 2/5] Updating packages and installing build dependencies...${COLOR_RESET}"
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev libudev-dev curl jq git

echo -e "${COLOR_GREEN}✔ System dependencies installed successfully.${COLOR_RESET}"

# ------------------------------------------------------------------------------
# STEP 3: INSTALL RUST TOOLCHAIN
# ------------------------------------------------------------------------------
echo -e "\n${COLOR_YELLOW}[Step 3/5] Installing official Rust toolchain (rustup)...${COLOR_RESET}"
if command -v cargo &> /dev/null; then
    echo -e "${COLOR_GREEN}✔ Rust/Cargo is already installed: $(cargo --version)${COLOR_RESET}"
else
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    echo -e "${COLOR_GREEN}✔ Rust successfully installed! Version: $(cargo --version)${COLOR_RESET}"
fi

# Ensure cargo is active in current session
export PATH="$HOME/.cargo/bin:$PATH"

# ------------------------------------------------------------------------------
# STEP 4: CREATE TEMPLATE CONFIG FILE
# ------------------------------------------------------------------------------
echo -e "\n${COLOR_YELLOW}[Step 4/5] Initializing configuration template (config.json)...${COLOR_RESET}"
CONFIG_FILE="config.json"

if [ -f "$CONFIG_FILE" ]; then
    echo -e "${COLOR_GREEN}✔ Existing config.json found.${COLOR_RESET}"
else
    cat <<EOT > config.json
{
  "rpc_http_url": "https://api.mainnet-beta.solana.com",
  "rpc_ws_url": "wss://api.mainnet-beta.solana.com",
  "wallet_private_key": "YOUR_PRIVATE_KEY_BASE58_HERE",
  "trade_amount_sol": 0.04,
  "max_active_trades": 3,
  "min_liquidity_usd": 1000.0,
  "min_score_threshold": 65,
  "max_slippage_percent": 25.0,
  "stop_loss_percent": 40.0,
  "take_profit_percent": 200.0,
  "priority_fee_lamports": 100000,
  "enable_honeypot_checks": true,
  "max_bundled_supply_percent": 30.0,
  "check_dev_history": true
}
EOT
    echo -e "${COLOR_GREEN}✔ config.json created. Open it after setup to insert your wallet key and RPC endpoints.${COLOR_RESET}"
fi

# ------------------------------------------------------------------------------
# STEP 5: COMPILE THE SNIPER BOT
# ------------------------------------------------------------------------------
echo -e "\n${COLOR_YELLOW}[Step 5/5] Compiling Rust Pump.fun Sniper Bot in Release mode...${COLOR_RESET}"
echo -e "${COLOR_CYAN}Note: Our Cargo.toml release profile is hard-optimized for 1GB Oracle Cloud VMs.${COLOR_RESET}"
echo -e "${COLOR_CYAN}It limits codegen tasks and maximizes RAM safety, preventing compile hangs.${COLOR_RESET}"

cargo build --release

echo -e "\n${COLOR_GREEN}======================================================================${COLOR_RESET}"
echo -e "${COLOR_GREEN}      🎉 EXCELLENT! SETUP AND COMPILATION COMPLETED SUCCESSFULLY 🎉      ${COLOR_RESET}"
echo -e "${COLOR_GREEN}======================================================================${COLOR_RESET}"

echo -e "\n${COLOR_YELLOW}👉 HOW TO RUN AND CONFIGURE THE BOT:${COLOR_RESET}"
echo -e "${COLOR_CYAN}1. Open 'config.json' and fill in your Solana Private Key (Base58) and fast RPC Endpoints (Helius / Quicknode / Chainstack etc.)${COLOR_RESET}"
echo -e "   Command to edit: ${COLOR_GREEN}nano config.json${COLOR_RESET}"
echo -e "${COLOR_CYAN}2. Launch the bot dashboard with:${COLOR_RESET}"
echo -e "   Command: ${COLOR_GREEN}./target/release/pumpfun_sniper_bot${COLOR_RESET}"
echo -e "${COLOR_CYAN}3. Use interactive hotkeys inside the terminal dashboard for absolute control:${COLOR_RESET}"
echo -e "   - Press [S] to Start/Pause the sniper stream."
echo -e "   - Press [M] to toggle Auto mode (buys high-rated coins) or Manual mode."
echo -e "   - Press [E] for EMERGENCY Sell-All."
echo -e "   - Press [U] / [D] to raise or lower buy sizes (+/- 0.01 SOL)."
echo -e "   - Press [+] / [-] to adjust minimum smart score filter (+/- 5 points)."
echo -e "   - Press [Q] to exit safely."

echo -e "\n${COLOR_YELLOW}Have a profitable sniping! 🚀${COLOR_RESET}\n"
