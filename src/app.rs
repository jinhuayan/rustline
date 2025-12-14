use std::io::{self, Write};
use std::path::PathBuf;

use crate::agent::Agent;
use crate::config::Config;
use crate::persistence::{
    session_manager::SessionManager,
    preference_manager::PreferenceManager,
    PersistenceError,
};

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    println!("Rustline - Local AI Agent CLI (async-ready)");
    println!("Type 'exit' or 'quit' to leave.\n");
    println!("Commands: :reset, :model <name>, :session <command>, :export <path>, :import <path>, :quit\n");

    // Initialize persistence managers
    let base_dir = get_persistence_base_dir();
    let session_manager = SessionManager::new(base_dir.clone())
        .map_err(|e| format!("Failed to initialize session manager: {}", e))?;
    let preference_manager = PreferenceManager::new(base_dir)
        .map_err(|e| format!("Failed to initialize preference manager: {}", e))?;

    // Create agent with persistence
    let mut agent = Agent::new_with_persistence(config, session_manager, preference_manager);
    
    // Load the current session or create a default one
    if let Err(e) = agent.load_session(None) {
        println!("Warning: Failed to load session: {}. Starting with empty session.", e);
    } else {
        if let Some(session_id) = agent.get_current_session_id() {
            println!("Loaded session: {}", session_id);
        }
    }

    run_cli_loop(agent).await
}

pub async fn run_with_agent(mut agent: Agent) -> Result<(), Box<dyn std::error::Error>> {
    println!("Rustline - Local AI Agent CLI (async-ready)");
    println!("Type 'exit' or 'quit' to leave.\n");
    println!("Commands: :reset, :model <name>, :session <command>, :export <path>, :import <path>, :quit\n");

    // Load the current session or create a default one
    if let Err(e) = agent.load_session(None) {
        println!("Warning: Failed to load session: {}. Starting with empty session.", e);
    } else {
        if let Some(session_id) = agent.get_current_session_id() {
            println!("Loaded session: {}", session_id);
        }
    }

    run_cli_loop(agent).await
}

async fn run_cli_loop(mut agent: Agent) -> Result<(), Box<dyn std::error::Error>> {

    loop {
        print!("User> ");
        io::stdout().flush().expect("failed to flush stdout");

        let mut input = String::new();
        let bytes_read = io::stdin()
            .read_line(&mut input)
            .expect("failed to read line");

        if bytes_read == 0 {
            println!("\nGoodbye.");
            break;
        }

        let trimmed = input.trim();

        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            println!("Bye!");
            break;
        }

        if trimmed.starts_with(':') {
            if trimmed.eq_ignore_ascii_case(":reset") {
                agent.reset();
                println!("Conversation history has been reset.");
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix(":model") {
                let new_model = rest.trim();
                if new_model.is_empty() {
                    println!("Usage: :model <model_name>");
                } else {
                    agent.set_model(new_model.to_string());
                    println!("Model switched to: {new_model}");
                }
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix(":session") {
                if handle_session_command(&mut agent, rest.trim()).await {
                    continue;
                } else {
                    continue;
                }
            }

            if let Some(rest) = trimmed.strip_prefix(":export") {
                let export_path = rest.trim();
                if export_path.is_empty() {
                    println!("Usage: :export <file_path>");
                } else {
                    match agent.export_data(&PathBuf::from(export_path)) {
                        Ok(()) => println!("Data exported successfully to: {}", export_path),
                        Err(e) => println!("Export failed: {}", e),
                    }
                }
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix(":import") {
                let import_path = rest.trim();
                if import_path.is_empty() {
                    println!("Usage: :import <file_path>");
                } else {
                    match agent.import_data(&PathBuf::from(import_path)) {
                        Ok(()) => println!("Data imported successfully from: {}", import_path),
                        Err(e) => println!("Import failed: {}", e),
                    }
                }
                continue;
            }

            println!("Unknown command: {trimmed}");
            println!("Available commands: :reset, :model <name>, :session <command>, :export <path>, :import <path>, :quit");
            continue;
        }

        let reply = agent.handle_message(trimmed).await?;
        println!("Rustline: {reply}");
        println!("---\n");
    }

    Ok(())
}

/// Get the base directory for persistence storage (fallback for legacy mode)
fn get_persistence_base_dir() -> PathBuf {
    // Use the same logic as Config::default() for consistency
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".rustline")
    } else if let Ok(userprofile) = std::env::var("USERPROFILE") {
        PathBuf::from(userprofile).join(".rustline")
    } else {
        PathBuf::from("./data")
    }
}

/// Handle session management commands
async fn handle_session_command(agent: &mut Agent, command: &str) -> bool {
    let parts: Vec<&str> = command.split_whitespace().collect();
    
    if parts.is_empty() {
        println!("Usage: :session <list|new|switch|delete|current> [args]");
        return true;
    }

    match parts[0] {
        "list" => {
            match agent.list_sessions() {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        println!("No sessions found.");
                    } else {
                        println!("Available sessions:");
                        for session in sessions {
                            let current_marker = if Some(&session.id) == agent.get_current_session_id().as_ref() {
                                " (current)"
                            } else {
                                ""
                            };
                            println!("  {} - {} messages, created: {}, last modified: {}{}",
                                session.id,
                                session.message_count,
                                session.created_at.format("%Y-%m-%d %H:%M:%S"),
                                session.last_modified.format("%Y-%m-%d %H:%M:%S"),
                                current_marker
                            );
                        }
                    }
                }
                Err(e) => println!("Failed to list sessions: {}", e),
            }
        }
        "new" => {
            let name = if parts.len() > 1 {
                Some(parts[1..].join(" "))
            } else {
                None
            };
            
            match agent.create_new_session(name.clone()) {
                Ok(session_id) => {
                    println!("Created new session: {} {}", session_id, 
                        name.map(|n| format!("({})", n)).unwrap_or_default());
                }
                Err(e) => println!("Failed to create session: {}", e),
            }
        }
        "switch" => {
            if parts.len() < 2 {
                println!("Usage: :session switch <session_id>");
            } else {
                let session_id = parts[1];
                match agent.switch_session(session_id) {
                    Ok(()) => println!("Switched to session: {}", session_id),
                    Err(e) => println!("Failed to switch session: {}", e),
                }
            }
        }
        "delete" => {
            if parts.len() < 2 {
                println!("Usage: :session delete <session_id>");
            } else {
                let session_id = parts[1];
                match agent.delete_session(session_id) {
                    Ok(()) => println!("Deleted session: {}", session_id),
                    Err(e) => println!("Failed to delete session: {}", e),
                }
            }
        }
        "current" => {
            if let Some(session_id) = agent.get_current_session_id() {
                println!("Current session: {}", session_id);
            } else {
                println!("No current session");
            }
        }
        _ => {
            println!("Unknown session command: {}", parts[0]);
            println!("Available session commands: list, new [name], switch <id>, delete <id>, current");
        }
    }
    
    true
}
