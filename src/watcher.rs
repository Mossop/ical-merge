use std::time::Duration;

use notify::{Config, Event, EventKind, PollWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::server::AppState;

/// Start watching the config file for changes
pub fn start_config_watcher(state: AppState) -> crate::error::Result<()> {
    start_config_watcher_with_interval(state, Duration::from_secs(2))
}

/// Start watching with custom poll interval (mainly for testing)
fn start_config_watcher_with_interval(
    state: AppState,
    poll_interval: Duration,
) -> crate::error::Result<()> {
    let config_path = state.config_path.as_ref().clone();
    let config_path_clone = config_path.clone();

    // Create channel for file events
    let (tx, mut rx) = mpsc::unbounded_channel();

    // Create PollWatcher with specified polling interval and content comparison
    let mut watcher = PollWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                // Check if this event is for our config file
                let is_our_file = event.paths.iter().any(|p| p == &config_path_clone);

                if is_our_file
                    && matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
                    )
                {
                    tracing::debug!("File event detected for config: {:?}", event);
                    let _ = tx.send(());
                }
            }
        },
        Config::default()
            .with_poll_interval(poll_interval)
            .with_compare_contents(true),
    )?;

    // Watch the parent directory to catch file changes reliably
    let watch_path = if config_path.is_file() {
        config_path.parent().unwrap_or(&config_path)
    } else {
        &config_path
    };

    watcher.watch(watch_path, RecursiveMode::NonRecursive)?;

    tracing::info!(
        "Started watching directory: {:?} for config file: {:?}",
        watch_path,
        config_path
    );

    // Spawn background task to handle reload events
    tokio::spawn(async move {
        // Keep watcher alive by moving it into the task
        let _watcher = watcher;

        while rx.recv().await.is_some() {
            tracing::debug!("Config file change detected, reloading...");

            match state.reload_config() {
                Ok(()) => {
                    tracing::info!("Configuration reloaded successfully");
                }
                Err(e) => {
                    tracing::error!("Failed to reload configuration: {}", e);
                    tracing::warn!("Continuing with previous configuration");
                }
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CalendarConfig, Config, ServerConfig, SourceConfig};
    use crate::fetcher::Fetcher;
    use std::collections::HashMap;
    use std::fs;
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn test_config_reload_on_file_change() {
        use std::env;
        use std::fs;

        // Use a unique temp file for this test
        let temp_dir = env::temp_dir();
        let test_id = format!("test-watcher-{}", std::process::id());
        let config_path = temp_dir.join(format!("{}.json", test_id));

        // Clean up any existing file
        let _ = fs::remove_file(&config_path);

        // Write initial config
        let mut calendars = HashMap::new();
        calendars.insert(
            "cal1".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
                    url: "https://example.com/test1.ics".to_string(),
                    filters: Default::default(),
                    modifiers: vec![],
                }],
            },
        );

        let config = Config {
            server: ServerConfig::default(),
            calendars: calendars.clone(),
        };

        fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        // Ensure file exists and is readable
        assert!(config_path.exists());

        // Create app state
        let fetcher = Fetcher::new().unwrap();
        let state = AppState::new(config, config_path.clone(), fetcher);

        // Start watcher with short poll interval for testing
        start_config_watcher_with_interval(state.clone(), Duration::from_millis(500)).unwrap();

        // Verify initial config
        {
            let config = state.config.read().unwrap();
            assert_eq!(config.calendars.len(), 1);
            assert!(config.calendars.contains_key("cal1"));
        }

        // Wait for first poll cycle to complete
        sleep(Duration::from_millis(600)).await;

        // Modify config - add a new calendar
        calendars.insert(
            "cal2".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
                    url: "https://example.com/test2.ics".to_string(),
                    filters: Default::default(),
                    modifiers: vec![],
                }],
            },
        );

        let new_config = Config {
            server: ServerConfig::default(),
            calendars: calendars.clone(),
        };

        // Write new config - with_compare_contents will detect the change
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&new_config).unwrap(),
        )
        .unwrap();

        // Wait for watcher to detect and reload (poll interval is 2s + processing time)
        // Poll until config is reloaded or timeout
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(10);
        let mut reloaded = false;

        while start.elapsed() < timeout {
            {
                let config = state.config.read().unwrap();
                if config.calendars.len() == 2 {
                    reloaded = true;
                    break;
                }
            }
            sleep(Duration::from_millis(500)).await;
        }

        assert!(reloaded, "Config was not reloaded within timeout");

        // Verify config was reloaded correctly
        {
            let config = state.config.read().unwrap();
            assert_eq!(config.calendars.len(), 2);
            assert!(config.calendars.contains_key("cal1"));
            assert!(config.calendars.contains_key("cal2"));
        }

        // Cleanup
        let _ = fs::remove_file(config_path);
    }

    #[tokio::test]
    async fn test_invalid_config_keeps_old_config() {
        use std::env;

        let temp_dir = env::temp_dir();
        let test_id = format!("test-watcher-invalid-{}", std::process::id());
        let config_path = temp_dir.join(format!("{}.json", test_id));

        // Clean up any existing file
        let _ = fs::remove_file(&config_path);

        // Write initial valid config
        let mut calendars = HashMap::new();
        calendars.insert(
            "cal1".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
                    url: "https://example.com/test1.ics".to_string(),
                    filters: Default::default(),
                    modifiers: vec![],
                }],
            },
        );

        let config = Config {
            server: ServerConfig::default(),
            calendars: calendars.clone(),
        };

        fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        // Create app state
        let fetcher = Fetcher::new().unwrap();
        let state = AppState::new(config, config_path.clone(), fetcher);

        // Start watcher with short poll interval for testing
        start_config_watcher_with_interval(state.clone(), Duration::from_millis(500)).unwrap();

        // Wait a bit for watcher to start
        sleep(Duration::from_millis(200)).await;

        // Write invalid JSON
        fs::write(&config_path, "{ invalid json }").unwrap();

        // Wait for watcher to detect change
        sleep(Duration::from_millis(1500)).await;

        // Verify old config is still there
        {
            let config = state.config.read().unwrap();
            assert_eq!(config.calendars.len(), 1);
            assert!(config.calendars.contains_key("cal1"));
        }

        // Cleanup
        let _ = fs::remove_file(config_path);
    }
}
