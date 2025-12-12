use std::env;

/// Runtime configuration for Rustline.
#[derive(Clone)]
pub struct Config {
    pub ollama_base_url: String, // default: "http://localhost:11434"
    pub model: String, // default: "gemma3"
    pub precheck_mode: String, // "strict" (default) or "assisted"
    pub confirm_before_tools: bool, // require confirmation before running tools
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

        Config {
            ollama_base_url,
            model,
            precheck_mode,
            confirm_before_tools,
        }
    }
}
