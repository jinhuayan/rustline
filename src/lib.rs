pub mod app;
pub mod agent;
pub mod ollama;
pub mod config;
pub mod tools;
pub mod ui;
pub mod persistence;

#[derive(Debug, Clone)]
pub enum PersistenceState {
    Enabled,
    Disabled,
    FailedFallback(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum InterfaceMode {
    Tui,
    Cli,
}

#[derive(Debug, Clone)]
pub struct ParsedArgs {
    pub interface_mode: InterfaceMode,
    pub session_command: Option<String>,
    pub target_session: Option<String>,
}

/// Parse command-line arguments
pub fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut interface_mode = InterfaceMode::Tui; // Default to TUI mode
    let mut session_command: Option<String> = None;
    let mut target_session: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--tui" => interface_mode = InterfaceMode::Tui,
            "--cli" => interface_mode = InterfaceMode::Cli,
            "--session" => {
                if i + 1 < args.len() {
                    target_session = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return Err("Error: --session requires a session ID".to_string());
                }
            }
            "--list-sessions" => session_command = Some("list".to_string()),
            "--new-session" => {
                if i + 1 < args.len() {
                    session_command = Some(format!("new:{}", args[i + 1]));
                    i += 1;
                } else {
                    session_command = Some("new".to_string());
                }
            }
            "--delete-session" => {
                if i + 1 < args.len() {
                    session_command = Some(format!("delete:{}", args[i + 1]));
                    i += 1;
                } else {
                    return Err("Error: --delete-session requires a session ID".to_string());
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => {
                if args[i].starts_with("--") {
                    return Err(format!("Unknown option: {}\nUse --help for usage information", args[i]));
                }
            }
        }
        i += 1;
    }

    Ok(ParsedArgs {
        interface_mode,
        session_command,
        target_session,
    })
}

/// Print help information
pub fn print_help() {
    println!("Rustline - Local AI Agent CLI");
    println!();
    println!("USAGE:");
    println!("    rustline [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --cli                    Run in CLI mode");
    println!("    --tui                    Run in Terminal UI mode (default)");
    println!("    --session <ID>           Start with specific session");
    println!("    --list-sessions          List all available sessions");
    println!("    --new-session [NAME]     Create a new session");
    println!("    --delete-session <ID>    Delete a session");
    println!("    -h, --help               Print this help message");
    println!();
    println!("By default, rustline starts in TUI mode. Use --cli to run in CLI mode.");
    println!();
    println!("ENVIRONMENT VARIABLES:");
    println!("    RUSTLINE_OLLAMA_URL           Ollama server URL (default: http://localhost:11434)");
    println!("    RUSTLINE_MODEL                Default model (default: gemma3)");
    println!("    RUSTLINE_PRECHECK_MODE        Precheck mode: strict|assisted (default: strict)");
    println!("    RUSTLINE_CONFIRM_TOOLS        Confirm before tools: true|false (default: true)");
    println!("    RUSTLINE_PERSISTENCE_ENABLED  Enable persistence: true|false (default: true)");
    println!("    RUSTLINE_DATA_DIR             Data directory (default: ~/.rustline)");
    println!("    RUSTLINE_AUTO_SAVE_INTERVAL   Auto-save interval in seconds (default: 30)");
    println!("    RUSTLINE_DEFAULT_SESSION_NAME Default session name");
}