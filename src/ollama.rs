use reqwest::Client;
use serde::{Deserialize, Serialize};

const OLLAMA_CHAT_URL: &str = "http://localhost:11434/api/chat";
const DEFAULT_MODEL: &str = "gemma3";

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

/// Send a single user message to Ollama and get back the response text.
pub async fn chat(
    client: &Client,
    user_input: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let req_body = ChatRequest {
        model: DEFAULT_MODEL.to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: user_input.to_string(),
        }],
        stream: false,
    };

    let resp = client
        .post(OLLAMA_CHAT_URL)
        .json(&req_body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama HTTP error {status}: {body_text}").into());
    }

    let chat_resp: ChatResponse = resp.json().await?;
    Ok(chat_resp.message.content)
}
