use tokio::time::{sleep, Duration};

pub struct Agent {
    name: String,
}

impl Agent {

    pub fn new() -> Self {
        Agent {
            name: "Rustline Agent".to_string(),
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

        let reply = format!("({}) I heard: {}", self.name, input);
        Ok(reply)
    }
}
