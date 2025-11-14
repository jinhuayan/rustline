use tokio::time::{sleep, Duration};
use reqwest::Client;

pub struct Agent {
    name: String,
    htttp: Client,
}

impl Agent {

    pub fn new() -> Self {
        Agent {
            name: "Rustline Agent".to_string(),
            htttp: Client::new(),
        }
    }

    pub async fn handle_message(
        &mut self,
        input: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if input.is_empty() {
            return Ok("You didn't type anything 🤔".to_string());
        }

        sleep(Duration::from_millis(100)).await;

        match crate::ollama::chat(&self.htttp, input).await {
            Ok(response) => Ok(response),
            Err(e) => {
                let err_msg = format!("Failed to get response from Ollama: {}\n I heard: {}", e, input);
                Ok(err_msg)
            }
        }
    }
}
