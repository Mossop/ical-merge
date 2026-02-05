//! Docker-based integration tests for config hot-reload functionality.
//!
//! These tests verify that config file changes are detected and applied correctly
//! when the app runs in a Docker container with bind-mounted config files.
//!
//! The tests use testcontainers to:
//! 1. Build the Docker image from the project's Dockerfile
//! 2. Start a container with a config file mounted from the host
//! 3. Make HTTP requests to verify the initial config works
//! 4. Modify the config file on the host filesystem
//! 5. Wait for the PollWatcher to detect the change (~2 seconds + buffer)
//! 6. Verify the server responds with updated configuration
//!
//! Note: These tests require Docker to be running and take longer to execute.
//!
//! Run with: cargo test --test docker_config_reload

use ical_merge::config::{CalendarConfig, Config, SourceConfig, Step};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Global mutex to ensure only one test builds the Docker image at a time
static BUILD_LOCK: Mutex<()> = Mutex::new(());

const INITIAL_CALENDAR: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:initial-event-1
DTSTART:20240115T100000Z
DTEND:20240115T110000Z
SUMMARY:Initial Event
END:VEVENT
END:VCALENDAR
"#;

const UPDATED_CALENDAR: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:updated-event-1
DTSTART:20240115T100000Z
DTEND:20240115T110000Z
SUMMARY:Updated Event
END:VEVENT
BEGIN:VEVENT
UID:updated-event-2
DTSTART:20240115T140000Z
DTEND:20240115T150000Z
SUMMARY:Second Updated Event
END:VEVENT
END:VCALENDAR
"#;

/// Test config hot-reload in Docker container with bind-mounted config file
#[tokio::test]
async fn test_config_reload_in_docker_container() {
    // Start mock calendar server that will be accessible from Docker container
    let mock_server = MockServer::start().await;

    // Set up initial calendar endpoint
    Mock::given(method("GET"))
        .and(path("/calendar.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(INITIAL_CALENDAR))
        .expect(1..)
        .mount(&mock_server)
        .await;

    // Create temporary directory for config file
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("config.json");

    // Get mock server URL
    // On Linux with host networking, use localhost directly
    // On Mac/Windows, use host.docker.internal
    let mock_url = if cfg!(target_os = "linux") {
        format!(
            "http://127.0.0.1:{}/calendar.ics",
            mock_server.address().port()
        )
    } else {
        format!(
            "http://host.docker.internal:{}/calendar.ics",
            mock_server.address().port()
        )
    };

    // Write initial config with one calendar
    let mut calendars = HashMap::new();
    calendars.insert(
        "test-cal".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: mock_url.clone(),
                steps: vec![],
            }],
            steps: vec![],
        },
    );

    let config = Config {
        calendars: calendars.clone(),
    };
    fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    // Build image if not already built
    ensure_docker_image_built();

    println!("Starting container...");

    // Create container with mounted config
    let mut image = GenericImage::new("ical-merge-test", "latest")
        .with_exposed_port(ContainerPort::Tcp(8080))
        .with_wait_for(WaitFor::message_on_stdout("Server listening"))
        .with_mount(testcontainers::core::Mount::bind_mount(
            config_path.to_str().unwrap(),
            "/app/config/config.json",
        ))
        .with_env_var("ICAL_MERGE_CONFIG", "/app/config/config.json")
        .with_env_var("RUST_LOG", "ical_merge=debug,tower_http=debug");

    // On Linux (GitHub Actions), use host networking for reliable host access
    // On Mac/Windows, use host.docker.internal which is built-in
    if cfg!(target_os = "linux") {
        image = image.with_network("host");
    }

    let container = image.start().await.expect("Failed to start container");

    // On Linux with host networking, the app is directly on port 8080
    // On Mac/Windows, we need to get the mapped port
    let base_url = if cfg!(target_os = "linux") {
        "http://127.0.0.1:8080".to_string()
    } else {
        let host_port = container
            .get_host_port_ipv4(8080)
            .await
            .expect("Failed to get host port");
        format!("http://127.0.0.1:{}", host_port)
    };
    println!("Container started, accessible at {}", base_url);

    // Give container a moment to fully initialize
    sleep(Duration::from_secs(2)).await;

    // Test 1: Verify initial config works
    println!("Testing initial config...");
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/ical/test-cal", base_url))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .expect("Failed to make initial request");

    let status = response.status();
    println!("Response status: {}", status);
    assert_eq!(status, 200, "Initial request should succeed");
    let body = response.text().await.unwrap();
    println!("Response body length: {}", body.len());
    println!("Response body preview: {}", &body[..body.len().min(200)]);
    assert!(
        body.contains("Initial Event"),
        "Should contain initial event. Body: {}",
        body
    );
    assert!(
        !body.contains("Updated Event"),
        "Should not contain updated event yet"
    );

    println!("Initial config verified ✓");

    // Test 2: Modify config to add a second calendar with modified steps
    println!("Modifying config file...");

    calendars.insert(
        "test-cal-modified".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: mock_url.clone(),
                steps: vec![Step::Replace {
                    pattern: "Initial".to_string(),
                    replacement: "Modified".to_string(),
                    field: "summary".to_string(),
                }],
            }],
            steps: vec![],
        },
    );

    let updated_config = Config { calendars };
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&updated_config).unwrap(),
    )
    .unwrap();

    println!("Config file modified, waiting for reload...");

    // Wait for config to be reloaded
    // PollWatcher interval is 2 seconds in production, plus some buffer for processing
    sleep(Duration::from_secs(5)).await;

    // Test 3: Verify new calendar is available
    println!("Testing reloaded config...");
    let response = client
        .get(format!("{}/ical/test-cal-modified", base_url))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .expect("Failed to make request to new calendar");

    assert_eq!(
        response.status(),
        200,
        "New calendar should be available after reload"
    );
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Modified Event"),
        "Should contain modified event with replacement applied"
    );
    assert!(
        !body.contains("Initial Event"),
        "Should not contain original text after replacement"
    );

    println!("Config reload verified ✓");

    // Test 4: Verify original calendar still works
    println!("Verifying original calendar still works...");
    let response = client
        .get(format!("{}/ical/test-cal", base_url))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .expect("Failed to make request to original calendar");

    assert_eq!(
        response.status(),
        200,
        "Original calendar should still work"
    );

    println!("All tests passed ✓");

    // Container will be automatically stopped and cleaned up when dropped
}

/// Test that the config reload works with source URL changes
#[tokio::test]
async fn test_docker_config_reload_with_url_change() {
    // Start two mock servers with different content
    let mock_server1 = MockServer::start().await;
    let mock_server2 = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/cal.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(INITIAL_CALENDAR))
        .mount(&mock_server1)
        .await;

    Mock::given(method("GET"))
        .and(path("/cal.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(UPDATED_CALENDAR))
        .mount(&mock_server2)
        .await;

    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("config.json");

    let mock_url1 = get_docker_accessible_url(mock_server1.address().port());
    let mock_url2 = get_docker_accessible_url(mock_server2.address().port());

    // Initial config pointing to first mock server
    let mut calendars = HashMap::new();
    calendars.insert(
        "dynamic".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: format!("{}/cal.ics", mock_url1),
                steps: vec![],
            }],
            steps: vec![],
        },
    );

    let config = Config {
        calendars: calendars.clone(),
    };
    fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    // Build image if not already built
    ensure_docker_image_built();

    println!("Starting container...");
    let mut image = GenericImage::new("ical-merge-test", "latest")
        .with_exposed_port(ContainerPort::Tcp(8080))
        .with_wait_for(WaitFor::message_on_stdout("Server listening"))
        .with_mount(testcontainers::core::Mount::bind_mount(
            config_path.to_str().unwrap(),
            "/app/config/config.json",
        ))
        .with_env_var("ICAL_MERGE_CONFIG", "/app/config/config.json")
        .with_env_var("RUST_LOG", "ical_merge=debug");

    // On Linux (GitHub Actions), use host networking for reliable host access
    if cfg!(target_os = "linux") {
        image = image.with_network("host");
    }

    let container = image.start().await.expect("Failed to start container");

    // On Linux with host networking, the app is directly on port 8080
    let base_url = if cfg!(target_os = "linux") {
        "http://127.0.0.1:8080".to_string()
    } else {
        let host_port = container.get_host_port_ipv4(8080).await.unwrap();
        format!("http://127.0.0.1:{}", host_port)
    };

    sleep(Duration::from_secs(2)).await;

    // Verify initial source
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/ical/dynamic", base_url))
        .send()
        .await
        .unwrap();
    let status = response.status();
    println!("Initial request status: {}", status);
    let body = response.text().await.unwrap();
    println!("Initial response body length: {}", body.len());
    println!(
        "Initial response body preview: {}",
        &body[..body.len().min(200)]
    );
    assert!(
        body.contains("Initial Event"),
        "Should contain Initial Event. Body: {}",
        body
    );

    // Update config to point to second mock server
    let mut calendars = HashMap::new();
    calendars.insert(
        "dynamic".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: format!("{}/cal.ics", mock_url2),
                steps: vec![],
            }],
            steps: vec![],
        },
    );

    fs::write(
        &config_path,
        serde_json::to_string_pretty(&Config { calendars }).unwrap(),
    )
    .unwrap();

    println!("Config updated, waiting for reload...");
    sleep(Duration::from_secs(5)).await;

    // Verify new source is being used
    let response = client
        .get(format!("{}/ical/dynamic", base_url))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Updated Event"),
        "Should now fetch from second server"
    );
    assert!(
        body.contains("Second Updated Event"),
        "Should have both events from updated calendar"
    );
}

// Helper functions

fn get_project_root() -> PathBuf {
    // Get the project root by going up from the tests directory
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn get_docker_accessible_url(port: u16) -> String {
    // On Linux with host networking, use localhost directly
    // On Mac/Windows, use host.docker.internal (built-in)
    if cfg!(target_os = "linux") {
        format!("http://127.0.0.1:{}", port)
    } else {
        format!("http://host.docker.internal:{}", port)
    }
}

fn ensure_docker_image_built() {
    // Use a lock to ensure only one test builds the image at a time
    let _lock = BUILD_LOCK.lock().unwrap();

    // Check if image already exists
    let check_output = std::process::Command::new("docker")
        .args(["images", "-q", "ical-merge-test:latest"])
        .output()
        .expect("Failed to check for Docker image");

    if !check_output.stdout.is_empty() {
        // Image exists, skip building
        return;
    }

    println!("Building Docker image...");
    let build_output = std::process::Command::new("docker")
        .args([
            "build",
            "-t",
            "ical-merge-test:latest",
            "-f",
            "Dockerfile",
            ".",
        ])
        .current_dir(get_project_root())
        .output()
        .expect("Failed to build Docker image");

    if !build_output.status.success() {
        panic!(
            "Docker build failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&build_output.stdout),
            String::from_utf8_lossy(&build_output.stderr)
        );
    }
}
