mod app;
mod agent;
mod ollama;
mod config;

use config::Config;

#[tokio::main]
async fn main() {
    let config = Config::load();
    if let Err(e) = app::run(config).await {
        eprintln!("Error: {e}");
    }
}