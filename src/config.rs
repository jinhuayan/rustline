use std::env;

/// Runtime configuration for Rustline.
#[derive(Clone)]
pub struct Config {
    pub ollama_base_url: String, // default: "http://localhost:11434"
    pub model: String, // default: "gemma3"
}

impl Config {
    pub fn load() -> Self {
        let ollama_base_url = env::var("RUSTLINE_OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());

        let model = env::var("RUSTLINE_MODEL")
            .unwrap_or_else(|_| "gemma3".to_string());

        Config {
            ollama_base_url,
            model,
        }
    }
}
