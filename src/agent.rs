use reqwest::Client;

use crate::ollama::{self, Message};
use crate::config::Config;

pub struct Agent {
    name: String,
    htttp: Client,
    history: Vec<Message>,
    config: Config,
}

impl Agent {

    pub fn new(config:Config) -> Self {
        let mut history = Vec::new();

        history.push(Message {
            role: "system".to_string(),
            content: "You are Rustline, a helpful local coding & CLI assistant.
            Answer concisely and clearly unless the user asks for more detail.".to_string(),
        });

        Agent {
            name: "Rustline Agent".to_string(),
            htttp: Client::new(),
            history,
            config,
        }
    }

    /// Reset the conversation history.
    pub fn reset(&mut self) {
        self.history.clear();
        self.history.push(Message {
            role: "system".to_string(),
            content: "You are Rustline, a helpful local coding & CLI assistant. \
                    Answer concisely and clearly unless the user asks for more detail."
                .to_string(),
        });
    }

    /// Change the model name at runtime.
    pub fn set_model(&mut self, model: String) {
        self.config.model = model;
    }

    pub async fn handle_message(
        &mut self,
        input: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if input.is_empty() {
            return Ok("You didn't type anything 🤔".to_string());
        }

        self.history.push(Message {
            role: "user".to_string(),
            content: input.to_string(),
        });

        let response = ollama::chat_with_history(
            &self.htttp, 
            &self.history,
            &self.config.ollama_base_url,
            &self.config.model,
        ).await;

        match response {
            Ok(reply) => {
                self.history.push(Message {
                    role: "assistant".to_string(),
                    content: reply.clone(),
                });
                Ok(reply)
            }
            Err(err) => {
                let error_msg = format!("Error communicating with Ollama API: {}", err);
                Ok(error_msg)
            }
        }
    }
}
