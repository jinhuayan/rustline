use reqwest::Client;
use tokio::time::{sleep, Duration};

use crate::config::Config;
use crate::ollama::{self, Message};
use crate::tools::{self, DynTool};

/// Core “brain” of Rustline.
/// Keeps config + conversation history, tools, and talks to Ollama.
pub struct Agent {
    name: String,
    http: Client,
    history: Vec<Message>,
    config: Config,
    tools: Vec<DynTool>,
}

impl Agent {
    /// Create a new agent with given config.
    pub fn new(config: Config) -> Self {
        let mut history = Vec::new();

        // Optional system prompt
        history.push(Message {
            role: "system".to_string(),
            content: "You are Rustline, a helpful local coding & CLI assistant. \
                      You may sometimes call local tools when the user explicitly asks \
                      with commands like !time or !echo."
                .to_string(),
        });

        Agent {
            name: "Rustline Agent".to_string(),
            http: Client::new(),
            history,
            config,
            tools: tools::default_tools(),
        }
    }

    /// Clear the conversation history, keeping the system prompt.
    pub fn reset(&mut self) {
        self.history.clear();
        self.history.push(Message {
            role: "system".to_string(),
            content: "You are Rustline, a helpful local coding & CLI assistant. \
                      You may sometimes call local tools when the user explicitly asks \
                      with commands like !time or !echo."
                .to_string(),
        });
    }

    /// Change the model name at runtime.
    pub fn set_model(&mut self, model: String) {
        self.config.model = model;
    }

    fn try_run_tool(&self, input: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if !input.starts_with('!') {
            return Ok(None);
        }

        // Strip leading '!' and split: first token = tool name, rest = args
        let rest = &input[1..];
        let mut parts = rest.splitn(2, ' ');
        let name_part = parts.next().unwrap_or("").to_lowercase();
        let args = parts.next().unwrap_or("").trim();

        // Special meta-tool: list available tools
        if name_part == "tools" || name_part == "help" {
            let mut out = String::from("Available tools:\n");
            for t in &self.tools {
                out.push_str(&format!("  !{} - {}\n", t.name(), t.description()));
            }
            return Ok(Some(out));
        }

        if name_part.is_empty() {
            return Ok(Some(
                "Usage: !<tool> [args]. Try !tools to list tools.".to_string(),
            ));
        }

        // Find matching tool
        if let Some(tool) = self
            .tools
            .iter()
            .find(|t| t.name().eq_ignore_ascii_case(&name_part))
        {
            let result = tool.invoke(args)?;
            Ok(Some(format!("[tool:{}]\n{}", name_part, result)))
        } else {
            Ok(Some(format!(
                "Unknown tool: {name}\nUse !tools to list available tools.",
                name = name_part
            )))
        }
    }

    /// Handle a single user message and return a reply.
    pub async fn handle_message(
        &mut self,
        input: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if input.is_empty() {
            return Ok("You didn't type anything 🤔".to_string());
        }

        // 0. Try tools first (commands starting with `!`)
        if let Some(tool_reply) = self.try_run_tool(input)? {
            // Optionally record the tool interaction in history
            self.history.push(Message {
                role: "user".to_string(),
                content: input.to_string(),
            });
            self.history.push(Message {
                role: "assistant".to_string(),
                content: tool_reply.clone(),
            });
            return Ok(tool_reply);
        }

        // Normal LLM flow
        sleep(Duration::from_millis(50)).await;

        // 1. Add user message to history
        self.history.push(Message {
            role: "user".to_string(),
            content: input.to_string(),
        });

        // 2. Call Ollama with full history, using current config
        let result = ollama::chat_with_history(
            &self.http,
            &self.config.ollama_base_url,
            &self.config.model,
            &self.history,
        )
        .await;

        match result {
            Ok(reply) => {
                // 3. On success, add assistant reply to history
                self.history.push(Message {
                    role: "assistant".to_string(),
                    content: reply.clone(),
                });
                Ok(reply)
            }
            Err(err) => {
                let fallback = format!(
                    "[Ollama error: {err}] (fallback)\n({}) I heard: {}",
                    self.name, input
                );

                self.history.push(Message {
                    role: "assistant".to_string(),
                    content: fallback.clone(),
                });

                Ok(fallback)
            }
        }
    }
}
