use reqwest::Client;
use crate::ollama::{self, Message};

pub struct Agent {
    name: String,
    htttp: Client,
    history: Vec<Message>,
}

impl Agent {

    pub fn new() -> Self {
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
        }
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

        let response = ollama::chat_with_history(&self.htttp, &self.history).await;

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
