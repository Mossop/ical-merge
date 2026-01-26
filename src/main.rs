use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use ical_merge::config::Config;
use ical_merge::error::Result;

#[derive(Parser)]
#[command(name = "ical-merge")]
#[command(about = "Merge and filter iCal calendars", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "config.json")]
    config: PathBuf,

    #[arg(long)]
    bind: Option<String>,

    #[arg(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ical_merge=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    let mut config = Config::load(&cli.config)?;
    config.validate()?;

    if let Some(bind) = cli.bind {
        config.server.bind_address = bind;
    }
    if let Some(port) = cli.port {
        config.server.port = port;
    }

    tracing::info!(
        "Starting server on {}:{}",
        config.server.bind_address,
        config.server.port
    );
    tracing::info!("Configured calendars: {:?}", config.calendars.keys());

    // TODO: Start server
    Ok(())
}
