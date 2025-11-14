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

#[derive(Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}


// Chat history function
pub async fn chat_with_history(
    client: &Client,
    messages: &[Message],
) -> Result<String, Box<dyn std::error::Error>> {
    let req_body = ChatRequest {
        model: DEFAULT_MODEL.to_string(),
        messages: messages.to_vec(),
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