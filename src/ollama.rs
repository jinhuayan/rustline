use reqwest::Client;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default = "Utc::now")]
    pub timestamp: DateTime<Utc>,
    #[serde(default = "generate_message_id")]
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_invocation: Option<ToolInvocation>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ToolInvocation {
    pub tool_name: String,
    pub input: String,
    pub output: String,
    pub success: bool,
}

/// Generate a unique message ID
fn generate_message_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Message {
    /// Create a new message with automatic timestamp and ID generation
    pub fn new(role: String, content: String) -> Self {
        Self {
            role,
            content,
            timestamp: Utc::now(),
            message_id: generate_message_id(),
            tool_invocation: None,
        }
    }
    
    /// Create a new message with tool invocation data
    pub fn new_with_tool(role: String, content: String, tool_invocation: ToolInvocation) -> Self {
        Self {
            role,
            content,
            timestamp: Utc::now(),
            message_id: generate_message_id(),
            tool_invocation: Some(tool_invocation),
        }
    }
    
    /// Create a message for compatibility with existing code (without metadata)
    pub fn simple(role: String, content: String) -> Self {
        Self::new(role, content)
    }
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
#[allow(dead_code)]
struct StreamChatResponse {
    message: StreamChatMessage,
    done: bool,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct StreamChatMessage {
    content: String,
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

/// Chat with Ollama using streaming and a callback for each token.
/// The callback receives each chunk of text as it arrives.
#[allow(dead_code)]
pub async fn chat_with_history_stream<F>(
    client: &Client,
    base_url: &str,
    model: &str,
    messages: &[Message],
    mut on_chunk: F,
) -> Result<String, Box<dyn std::error::Error>>
where
    F: FnMut(&str),
{
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let req_body = ChatRequest {
        model: model.to_string(),
        messages: messages.to_vec(),
        stream: true, 
    };

    let mut resp = client.post(&url).json(&req_body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama HTTP error {status}: {body_text}").into());
    }

    // Use the reqwest streaming API
    let mut full_content = String::new();
    let mut buffer = String::new();

    while let Some(chunk_result) = resp.chunk().await? {
        let text = String::from_utf8_lossy(&chunk_result);
        buffer.push_str(&text);
        
        // Parse each complete line as JSON
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();
            
            if line.trim().is_empty() {
                continue;
            }
            
            if let Ok(stream_resp) = serde_json::from_str::<StreamChatResponse>(&line) {
                if !stream_resp.message.content.is_empty() {
                    on_chunk(&stream_resp.message.content);
                    full_content.push_str(&stream_resp.message.content);
                }
                
                if stream_resp.done {
                    return Ok(full_content);
                }
            }
        }
    }

    Ok(full_content)
}

/// Single-turn helper for ReAct: supply a full prompt string.
pub async fn chat_single_turn(
    client: &Client,
    base_url: &str,
    model: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let messages = vec![Message::new("user".to_string(), prompt.to_string())];

    chat_with_history(client, base_url, model, &messages).await
}

/// Single-turn streaming helper for ReAct.
#[allow(dead_code)]
pub async fn chat_single_turn_stream<F>(
    client: &Client,
    base_url: &str,
    model: &str,
    prompt: &str,
    on_chunk: F,
) -> Result<String, Box<dyn std::error::Error>>
where
    F: FnMut(&str),
{
    let messages = vec![Message::new("user".to_string(), prompt.to_string())];

    chat_with_history_stream(client, base_url, model, &messages, on_chunk).await
}


