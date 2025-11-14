use reqwest::Client;
use serde::{Deserialize, Serialize};


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
    base_url: &str,
    model: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("{}/api/chat", base_url);
    let req_body = ChatRequest {
        model: model.to_string(),
        messages: messages.to_vec(),
        stream: false,
    };

    let resp = client
        .post(url)
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