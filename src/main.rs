mod agent_ruler;
mod cli;

#[tokio::main]
async fn main() {
    if let Err(err) = agent_ruler::run().await {
        eprintln!("error: {err:?}");
        std::process::exit(1);
    }
}
