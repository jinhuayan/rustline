use std::io::{self, Write};

use crate::agent::Agent;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("Rustline - Local AI Agent CLI (async-ready)");
    println!("Type 'exit' or 'quit' to leave.\n");

    let mut agent = Agent::new();

    loop {
        print!("rustline> ");
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

        let reply = agent.handle_message(trimmed).await?;
        println!("{reply}");
    }

    Ok(())
}
