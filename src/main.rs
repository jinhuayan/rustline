mod app;
mod agent;

#[tokio::main]
async fn main() {
    if let Err(e) = app::run().await {
        eprintln!("Error: {e}");
    }
}