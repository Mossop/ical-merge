use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use ical_merge::config::Config;
use ical_merge::error::Result;
use ical_merge::fetcher::Fetcher;
use ical_merge::merge::merge_calendars;
use ical_merge::server::{AppState, create_router};
use ical_merge::watcher::start_config_watcher;

#[derive(Parser)]
#[command(name = "ical-merge")]
#[command(about = "Merge and filter iCal calendars", long_about = None)]
struct Cli {
    #[arg(short, long, env = "ICAL_MERGE_CONFIG", default_value = "config.json")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the web server (default)
    Serve {
        #[arg(long, env = "ICAL_MERGE_BIND")]
        bind: Option<String>,

        #[arg(short, long, env = "ICAL_MERGE_PORT")]
        port: Option<u16>,
    },
    /// Show merged events for a calendar
    Show {
        /// Calendar ID from config
        calendar_id: String,
    },
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

    match cli.command.unwrap_or(Command::Serve {
        bind: None,
        port: None,
    }) {
        Command::Serve { bind, port } => run_serve(cli.config, bind, port).await,
        Command::Show { calendar_id } => run_show(cli.config, calendar_id).await,
    }
}

async fn run_serve(config_path: PathBuf, bind: Option<String>, port: Option<u16>) -> Result<()> {
    let mut config = Config::load(&config_path)?;
    config.validate()?;

    if let Some(bind) = bind {
        config.server.bind_address = bind;
    }
    if let Some(port) = port {
        config.server.port = port;
    }

    let bind_addr = format!("{}:{}", config.server.bind_address, config.server.port);

    tracing::info!("Starting server on {}", bind_addr);
    tracing::info!(
        "Configured calendars: {:?}",
        config.calendars.keys().collect::<Vec<_>>()
    );

    let fetcher = Fetcher::new()?;
    let state = AppState::new(config, config_path.clone(), fetcher);
    let app = create_router(state.clone());

    // Start config file watcher
    start_config_watcher(state.clone())?;
    tracing::info!("Config file watcher started");

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("Server listening on {}", bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_show(config_path: PathBuf, calendar_id: String) -> Result<()> {
    let config = Config::load(&config_path)?;
    config.validate()?;

    let calendar_config = config
        .calendars
        .get(&calendar_id)
        .ok_or_else(|| ical_merge::error::Error::CalendarNotFound(calendar_id.clone()))?;

    let fetcher = Fetcher::new()?;
    let result = merge_calendars(calendar_config, &fetcher).await?;

    // Report any errors
    for (url, error) in &result.errors {
        eprintln!("Error fetching {}: {}", url, error);
    }

    // Display events
    if result.events.is_empty() {
        println!("No events found");
        return Ok(());
    }

    for event in result.events {
        let summary = event.summary().unwrap_or("<no summary>");
        let start = event
            .start()
            .map(|dt| format_date_time(&dt))
            .unwrap_or_else(|| "<no start>".to_string());
        let end = event
            .end()
            .map(|dt| format_date_time(&dt))
            .unwrap_or_else(|| "<no end>".to_string());

        println!("{} - {}: {}", start, end, summary);
    }

    Ok(())
}

fn format_date_time(dt: &icalendar::DatePerhapsTime) -> String {
    use icalendar::DatePerhapsTime;

    match dt {
        DatePerhapsTime::DateTime(dt) => match dt {
            icalendar::CalendarDateTime::Floating(naive) => {
                naive.format("%Y-%m-%d %H:%M:%S").to_string()
            }
            icalendar::CalendarDateTime::Utc(utc) => {
                utc.format("%Y-%m-%d %H:%M:%S UTC").to_string()
            }
            icalendar::CalendarDateTime::WithTimezone { date_time, tzid } => {
                format!("{} ({})", date_time.format("%Y-%m-%d %H:%M:%S"), tzid)
            }
        },
        DatePerhapsTime::Date(date) => date.format("%Y-%m-%d").to_string(),
    }
}
