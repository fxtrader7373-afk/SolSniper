use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    style::{self, Color, Stylize},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct DashboardStats {
    pub status: String,               // "RUNNING" or "PAUSED"
    pub balance_sol: f64,
    pub total_scanned: u32,
    pub total_trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub win_rate: f64,
    pub net_pnl_sol: f64,
    pub active_positions: usize,
    pub current_capital_sol: f64,
    pub rpc_latency_ms: u64,
    pub sniper_mode: String,          // "AUTO" or "MANUAL"
    pub min_score_threshold: u32,
    pub max_slippage_percent: f64,
    pub last_sniped_token: String,
}

pub enum DashboardCommand {
    ToggleStatus,
    ToggleMode,
    EmergencySellAll,
    IncreaseCapital,
    DecreaseCapital,
    IncreaseScore,
    DecreaseScore,
    Quit,
}

pub struct TerminalDashboard {
    stats: Arc<Mutex<DashboardStats>>,
    logs: Arc<Mutex<Vec<String>>>,
    cmd_sender: mpsc::Sender<DashboardCommand>,
}

impl TerminalDashboard {
    pub fn new(
        stats: Arc<Mutex<DashboardStats>>,
        logs: Arc<Mutex<Vec<String>>>,
        cmd_sender: mpsc::Sender<DashboardCommand>,
    ) -> Self {
        Self {
            stats,
            logs,
            cmd_sender,
        }
    }

    /// Setup the terminal for interactive dashboard mode
    pub fn setup_terminal() -> Result<(), io::Error> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
        Ok(())
    }

    /// Restore standard terminal mode
    pub fn restore_terminal() -> Result<(), io::Error> {
        terminal::disable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, LeaveAlternateScreen, cursor::Show)?;
        Ok(())
    }

    /// Main loop to render the dashboard and listen for keyboard controls
    pub async fn run(self) -> Result<(), io::Error> {
        let mut stdout = io::stdout();
        let mut interval = tokio::time::interval(Duration::from_millis(300)); // 300ms refresh rate

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.render(&mut stdout)?;
                }
                // Async keyboard event polling
                Ok(has_event) = tokio::task::spawn_blocking(|| event::poll(Duration::from_millis(50))) => {
                    if has_event? {
                        if let Event::Key(key_event) = event::read()? {
                            match key_event.code {
                                KeyCode::Char('s') | KeyCode::Char('S') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::ToggleStatus).await;
                                }
                                KeyCode::Char('m') | KeyCode::Char('M') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::ToggleMode).await;
                                }
                                KeyCode::Char('e') | KeyCode::Char('E') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::EmergencySellAll).await;
                                }
                                KeyCode::Char('u') | KeyCode::Char('U') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::IncreaseCapital).await;
                                }
                                KeyCode::Char('d') | KeyCode::Char('D') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::DecreaseCapital).await;
                                }
                                KeyCode::Char('+') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::IncreaseScore).await;
                                }
                                KeyCode::Char('-') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::DecreaseScore).await;
                                }
                                KeyCode::Char('q') | KeyCode::Char('Q') => {
                                    let _ = self.cmd_sender.send(DashboardCommand::Quit).await;
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn render(&self, stdout: &mut io::Stdout) -> Result<(), io::Error> {
        // Clear terminal screen and reset cursor position
        execute!(stdout, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;

        let stats = self.stats.lock().unwrap().clone();
        let logs = self.logs.lock().unwrap().clone();

        // 1. Draw Title
        let border_line = "═".repeat(80);
        writeln!(stdout, "{}", border_line.cyan())?;
        writeln!(
            stdout,
            "          ⚡ PUMPFUN ADVANCED SNIPER BOT - REALTIME DASHBOARD ⚡          "
                .bold()
                .green()
        )?;
        writeln!(stdout, "{}", border_line.cyan())?;

        // 2. Render Live Metrics (15 key data metrics)
        let status_color = if stats.status == "RUNNING" {
            "RUNNING".bold().green()
        } else {
            "PAUSED".bold().red()
        };

        let mode_color = if stats.sniper_mode == "AUTO" {
            "AUTO".bold().magenta()
        } else {
            "MANUAL".bold().yellow()
        };

        let pnl_color = if stats.net_pnl_sol >= 0.0 {
            format!("+{:.4} SOL", stats.net_pnl_sol).bold().green()
        } else {
            format!("{:.4} SOL", stats.net_pnl_sol).bold().red()
        };

        writeln!(
            stdout,
            "  [{}] Status: {}   │  [{}] Mode: {}   │  [{}] Active Trades: {}",
            "1".yellow(),
            status_color,
            "2".yellow(),
            mode_color,
            "3".yellow(),
            stats.active_positions.to_string().cyan()
        )?;

        writeln!(
            stdout,
            "  [{}] Sol Balance: {:.4} SOL │  [{}] Capital limit: {:.4} SOL │  [{}] Net PnL: {}",
            "4".yellow(),
            stats.balance_sol.to_string().cyan(),
            "5".yellow(),
            stats.current_capital_sol.to_string().cyan(),
            "6".yellow(),
            pnl_color
        )?;

        writeln!(
            stdout,
            "  [{}] Total Scanned: {}     │  [{}] Executed: {}         │  [{}] Win Rate: {:.1}%",
            "7".yellow(),
            stats.total_scanned.to_string().cyan(),
            "8".yellow(),
            stats.total_trades.to_string().cyan(),
            "9".yellow(),
            stats.win_rate
        )?;

        writeln!(
            stdout,
            "  [{}] Wins / Losses: {}/{}    │  [{}] Min Score: {}         │  [{}] Max Slippage: {:.1}%",
            "10".yellow(),
            format!("{}/{}", stats.wins, stats.losses).cyan(),
            "11".yellow(),
            stats.min_score_threshold.to_string().cyan(),
            "12".yellow(),
            stats.max_slippage_percent
        )?;

        writeln!(
            stdout,
            "  [{}] RPC Latency: {} ms   │  [{}] Last Sniped: {}     │  [{}] VM Ram Limit: 1GB",
            "13".yellow(),
            stats.rpc_latency_ms.to_string().cyan(),
            "14".yellow(),
            if stats.last_sniped_token.is_empty() { "None".to_string() } else { stats.last_sniped_token.clone() }.yellow(),
            "15".yellow()
        )?;

        writeln!(stdout, "{}", "─".repeat(80).cyan())?;

        // 3. Render Dashboard Controls Panel
        writeln!(stdout, "  {}", "KEYBOARD CONTROLS:".bold().white())?;
        writeln!(
            stdout,
            "   [{}] Toggle Start/Pause │ [{}] Toggle Auto/Manual │ [{}] EMERGENCY SELL ALL",
            "S".green().bold(),
            "M".magenta().bold(),
            "E".red().bold()
        )?;
        writeln!(
            stdout,
            "   [{}] Increase Capital   │ [{}] Decrease Capital  │ [{}/{}] Adjust Min Score",
            "U".yellow().bold(),
            "D".yellow().bold(),
            "+".cyan().bold(),
            "-".cyan().bold()
        )?;
        writeln!(stdout, "   [{}] Exit safely", "Q".red().bold())?;

        writeln!(stdout, "{}", "═".repeat(80).cyan())?;

        // 4. Render Live Console Logs
        writeln!(stdout, "  {}", "LIVE EVENT LOGGER:".bold().white())?;
        let display_logs = if logs.len() > 8 {
            &logs[logs.len() - 8..]
        } else {
            &logs[..]
        };

        for log in display_logs {
            writeln!(stdout, "    {}", log)?;
        }

        stdout.flush()?;
        Ok(())
    }
}
