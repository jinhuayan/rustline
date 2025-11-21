use std::io::{self, Write};

use crate::agent::Agent;
use crate::config::Config;

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    println!("Rustline - Local AI Agent CLI (async-ready)");
    println!("Type 'exit' or 'quit' to leave.\n");
    println!("Commands: :reset, :model <name>, :quit\n");

    let mut agent = Agent::new(config);

    loop {
        print!("User> ");
        io::stdout().flush().expect("failed to flush stdout");

        let mut input = String::new();
        let bytes_read = io::stdin()
            .read_line(&mut input)
            .expect("failed to read line");

        if bytes_read == 0 {
            println!("\nGoodbye.");
            break;
        }

        let trimmed = input.trim();

        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            println!("Bye!");
            break;
        }

        if trimmed.starts_with(':') {
            if trimmed.eq_ignore_ascii_case(":reset") {
                agent.reset();
                println!("Conversation history has been reset.");
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix(":model") {
                let new_model = rest.trim();
                if new_model.is_empty() {
                    println!("Usage: :model <model_name>");
                } else {
                    agent.set_model(new_model.to_string());
                    println!("Model switched to: {new_model}");
                }
                continue;
            }

            println!("Unknown command: {trimmed}");
            println!("Available commands: :reset, :model <name>, :quit");
            continue;
        }

        let reply = agent.handle_message(trimmed).await?;
        println!("Rustline: {reply}");
        println!("---\n");
    }

    Ok(())
}
