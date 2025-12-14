use std::env;
use std::path::PathBuf;

/// Runtime configuration for Rustline.
#[derive(Clone)]
pub struct Config {
    pub ollama_base_url: String, // default: "http://localhost:11434"
    pub model: String, // default: "gemma3"
    pub precheck_mode: String, // "strict" (default) or "assisted"
    pub confirm_before_tools: bool, // require confirmation before running tools
    // Persistence settings
    pub persistence_enabled: bool, // default: true
    pub data_dir: PathBuf, // default: ~/.rustline or ./data
    pub auto_save_interval: Option<u64>, // seconds, default: Some(30)
    pub default_session_name: Option<String>, // default: None
    // ReAct loop settings
    pub react_max_iterations: u32, // default: 3
    pub react_iteration_timeout: Option<u64>, // seconds, default: Some(15)
}

impl Config {
    pub fn load() -> Self {
        let ollama_base_url = env::var("RUSTLINE_OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());

        let model = env::var("RUSTLINE_MODEL")
            .unwrap_or_else(|_| "gemma3".to_string());

        let precheck_mode = env::var("RUSTLINE_PRECHECK_MODE")
            .unwrap_or_else(|_| "strict".to_string());

        let confirm_before_tools = env::var("RUSTLINE_CONFIRM_TOOLS")
            .map(|v| {
                let l = v.to_lowercase();
                l == "1" || l == "true" || l == "yes" || l == "on"
            })
            .unwrap_or(true);

        // Persistence settings
        let persistence_enabled = env::var("RUSTLINE_PERSISTENCE_ENABLED")
            .map(|v| {
                let l = v.to_lowercase();
                l == "1" || l == "true" || l == "yes" || l == "on"
            })
            .unwrap_or(true);

        let data_dir = env::var("RUSTLINE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                // Default to ~/.rustline if HOME is available, otherwise ./data
                if let Ok(home) = env::var("HOME") {
                    PathBuf::from(home).join(".rustline")
                } else if let Ok(userprofile) = env::var("USERPROFILE") {
                    PathBuf::from(userprofile).join(".rustline")
                } else {
                    PathBuf::from("./data")
                }
            });

        let auto_save_interval = env::var("RUSTLINE_AUTO_SAVE_INTERVAL")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or(Some(30)); // Default to 30 seconds

        let default_session_name = env::var("RUSTLINE_DEFAULT_SESSION_NAME").ok();

        // ReAct loop settings
        let react_max_iterations = env::var("RUSTLINE_REACT_MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(3); // Default to 3 iterations

        let react_iteration_timeout = env::var("RUSTLINE_REACT_ITERATION_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or(Some(15)); // Default to 15 seconds

        Config {
            ollama_base_url,
            model,
            precheck_mode,
            confirm_before_tools,
            persistence_enabled,
            data_dir,
            auto_save_interval,
            default_session_name,
            react_max_iterations,
            react_iteration_timeout,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".rustline")
        } else if let Ok(userprofile) = std::env::var("USERPROFILE") {
            PathBuf::from(userprofile).join(".rustline")
        } else {
            PathBuf::from("./data")
        };

        Config {
            ollama_base_url: "http://localhost:11434".to_string(),
            model: "gemma3".to_string(),
            precheck_mode: "strict".to_string(),
            confirm_before_tools: true,
            persistence_enabled: true,
            data_dir,
            auto_save_interval: Some(30),
            default_session_name: None,
            react_max_iterations: 3,
            react_iteration_timeout: Some(15),
        }
    }
}
