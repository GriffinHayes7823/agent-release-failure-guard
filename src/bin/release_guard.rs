use agent_release_failure_guard::{
    infrai_errors::InfraiErrors,
    release_guard::{evaluate_and_track, BuildEvent},
};
use std::{env, process};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("release guard: {error}");
        process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let input = env::args().nth(1).ok_or("pass one JSON build event")?;
    let event: BuildEvent = serde_json::from_str(&input)?;
    let errors = InfraiErrors::from_env()?;
    let (operation, _receipt) = evaluate_and_track(&errors, event).await?;
    println!("{}", serde_json::to_string(&operation)?);
    Ok(())
}
