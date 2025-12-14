mod app;
mod agent;
mod ollama;
mod config;
mod tools;
mod ui;
mod persistence;

use config::Config;
use agent::Agent;
use persistence::{session_manager::SessionManager, preference_manager::PreferenceManager};
use std::env;

use rustline::{PersistenceState, InterfaceMode, parse_args};

#[tokio::main]
async fn main() {
    let config = Config::load();

    // Parse command-line arguments
    let args: Vec<String> = env::args().collect();
    let parsed_args = match parse_args(&args) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // Handle session management commands first
    if let Some(cmd) = parsed_args.session_command {
        if let Err(e) = handle_session_command(&config, &cmd).await {
            eprintln!("Session command failed: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // Run the application
    match parsed_args.interface_mode {
        InterfaceMode::Tui => run_tui_mode(config, parsed_args.target_session).await,
        InterfaceMode::Cli => run_cli_mode(config, parsed_args.target_session).await,
    }
}

async fn run_tui_mode(config: Config, target_session: Option<String>) {
    let persistence_state = if config.persistence_enabled {
        match initialize_persistence_managers(&config) {
            Ok((mut session_manager, preference_manager)) => {
                // Switch to target session if specified
                if let Some(session_id) = target_session {
                    if let Err(e) = session_manager.switch_session(&session_id) {
                        eprintln!("Warning: Failed to switch to session '{}': {}", session_id, e);
                    }
                }
                
                let agent = Agent::new_with_persistence(config.clone(), session_manager, preference_manager);
                if let Err(e) = ui::run_tui_with_persistence_state(agent, PersistenceState::Enabled).await {
                    eprintln!("TUI Error: {e}");
                }
                return;
            }
            Err(e) => {
                eprintln!("Failed to initialize persistence for TUI mode: {}", e);
                eprintln!("Falling back to non-persistent mode with clear user feedback...");
                PersistenceState::FailedFallback(e.to_string())
            }
        }
    } else {
        PersistenceState::Disabled
    };

    // Run in non-persistent mode with persistence state information
    let agent = Agent::new(config);
    if let Err(e) = ui::run_tui_with_persistence_state(agent, persistence_state).await {
        eprintln!("TUI Error: {e}");
    }
}

async fn run_cli_mode(config: Config, target_session: Option<String>) {
    if config.persistence_enabled {
        match initialize_persistence_managers(&config) {
            Ok((mut session_manager, preference_manager)) => {
                // Switch to target session if specified
                if let Some(session_id) = target_session {
                    if let Err(e) = session_manager.switch_session(&session_id) {
                        eprintln!("Warning: Failed to switch to session '{}': {}", session_id, e);
                    }
                }
                
                let agent = Agent::new_with_persistence(config, session_manager, preference_manager);
                if let Err(e) = app::run_with_agent(agent).await {
                    eprintln!("Error: {e}");
                }
            }
            Err(e) => {
                eprintln!("Failed to initialize persistence for CLI mode: {}", e);
                eprintln!("Falling back to non-persistent mode...");
                if let Err(e) = app::run(config).await {
                    eprintln!("Error: {e}");
                }
            }
        }
    } else {
        if let Err(e) = app::run(config).await {
            eprintln!("Error: {e}");
        }
    }
}

/// Initialize persistence managers using configuration
fn initialize_persistence_managers(
    config: &Config,
) -> Result<(SessionManager, PreferenceManager), Box<dyn std::error::Error>> {
    let session_manager = SessionManager::new(config.data_dir.clone())
        .map_err(|e| format!("Failed to initialize session manager: {}", e))?;
    let preference_manager = PreferenceManager::new(config.data_dir.clone())
        .map_err(|e| format!("Failed to initialize preference manager: {}", e))?;
    
    Ok((session_manager, preference_manager))
}

/// Handle session management commands
async fn handle_session_command(config: &Config, command: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session_manager, _) = initialize_persistence_managers(config)?;
    
    match command {
        "list" => {
            let sessions = session_manager.list_sessions()?;
            if sessions.is_empty() {
                println!("No sessions found.");
            } else {
                println!("Available sessions:");
                for session in sessions {
                    let name = session.name.unwrap_or_else(|| "Unnamed".to_string());
                    println!("  {} - {} (created: {}, messages: {})", 
                        session.id, name, 
                        session.created_at.format("%Y-%m-%d %H:%M:%S"),
                        session.message_count);
                }
            }
        }
        cmd if cmd.starts_with("new") => {
            let name = if cmd.len() > 4 {
                Some(cmd[4..].to_string())
            } else {
                None
            };
            let session_id = session_manager.create_session(name.clone())?;
            let display_name = name.unwrap_or_else(|| "Unnamed".to_string());
            println!("Created new session: {} ({})", session_id, display_name);
        }
        cmd if cmd.starts_with("delete:") => {
            let session_id = &cmd[7..];
            session_manager.delete_session(session_id)?;
            println!("Deleted session: {}", session_id);
        }
        _ => {
            return Err(format!("Unknown session command: {}", command).into());
        }
    }
    
    Ok(())
}



