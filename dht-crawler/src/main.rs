mod cli;
mod crawler;
mod discovery;
mod fetch;
mod net;
mod purge;
mod query;
mod stats;
mod storage;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => {
            init_tracing(&args.log);
            crawler::run(args).await
        }
        Command::Query(args) => query::query(args),
        Command::Purge(args) => purge::purge(&args),
    }
}

fn init_tracing(log: &Option<String>) {
    let filter = match log {
        Some(filter) => EnvFilter::new(filter),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
