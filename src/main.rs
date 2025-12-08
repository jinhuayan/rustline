mod app;
mod agent;
mod ollama;
mod config;
mod tools;
mod ui;

use config::Config;
use agent::Agent;
use std::env;

#[tokio::main]
async fn main() {
    let config = Config::load();
    let agent = Agent::new(config.clone());

    // Check for --tui flag
    let args: Vec<String> = env::args().collect();
    let use_tui = args.iter().any(|arg| arg == "--tui");

    if use_tui {
        // Run TUI mode
        if let Err(e) = ui::run_tui(agent).await {
            eprintln!("TUI Error: {e}");
        }
    } else {
        // Run legacy CLI mode
        if let Err(e) = app::run(config).await {
            eprintln!("Error: {e}");
        }
    }
}