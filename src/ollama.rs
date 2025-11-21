use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

/// Chat with Ollama using a history of messages.
pub async fn chat_with_history(
    client: &Client,
    base_url: &str,
    model: &str,
    messages: &[Message],
) -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let req_body = ChatRequest {
        model: model.to_string(),
        messages: messages.to_vec(),
        stream: false,
    };

    let resp = client.post(&url).json(&req_body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama HTTP error {status}: {body_text}").into());
    }

    let chat_resp: ChatResponse = resp.json().await?;
    Ok(chat_resp.message.content)
}

/// Single-turn helper for ReAct: supply a full prompt string.
pub async fn chat_single_turn(
    client: &Client,
    base_url: &str,
    model: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let messages = vec![Message {
        role: "user".to_string(),
        content: prompt.to_string(),
    }];

    chat_with_history(client, base_url, model, &messages).await
}
