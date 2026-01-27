use axum::body::Body;
use axum::http::{Request, StatusCode};
use ical_merge::config::{CalendarConfig, Config, MatchMode, ServerConfig, SourceConfig, Step};
use ical_merge::fetcher::Fetcher;
use ical_merge::ical::parse_calendar;
use ical_merge::merge::merge_calendars;
use ical_merge::server::{AppState, create_router};
use std::collections::HashMap;
use tower::util::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WORK_CALENDAR: &str = include_str!("fixtures/work.ics");
const HOLIDAYS_CALENDAR: &str = include_str!("fixtures/holidays.ics");
const PERSONAL_CALENDAR: &str = include_str!("fixtures/personal.ics");

#[tokio::test]
async fn test_full_flow_fetch_filter_modify_merge_serve() {
    let mock_server = MockServer::start().await;

    // Mount work and holidays calendars
    Mock::given(method("GET"))
        .and(path("/work.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(WORK_CALENDAR))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/holidays.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(HOLIDAYS_CALENDAR))
        .mount(&mock_server)
        .await;

    // Configure calendar with:
    // - Steps: deny optional, allow meetings, replace "Meeting:" with "[WORK]"
    let mut calendars = HashMap::new();
    calendars.insert(
        "combined-work".to_string(),
        CalendarConfig {
            sources: vec![
                SourceConfig::Url {
                    url: format!("{}/work.ics", mock_server.uri()),
                    steps: vec![
                        Step::Deny {
                            patterns: vec!["(?i)optional".to_string()],
                            mode: MatchMode::Any,
                            fields: vec!["summary".to_string()],
                        },
                        Step::Allow {
                            patterns: vec!["(?i)meeting".to_string()],
                            mode: MatchMode::Any,
                            fields: vec!["summary".to_string(), "description".to_string()],
                        },
                        Step::Replace {
                            pattern: "^Meeting:".to_string(),
                            replacement: "[WORK]".to_string(),
                            field: "summary".to_string(),
                        },
                    ],
                },
                SourceConfig::Url {
                    url: format!("{}/holidays.ics", mock_server.uri()),
                    steps: vec![],
                },
            ],
            steps: vec![],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let fetcher = Fetcher::new().unwrap();
    let config_path = std::env::temp_dir().join("test-integration-config.json");
    let state = AppState::new(config, config_path, fetcher);
    let app = create_router(state);

    // Make request
    let request = Request::builder()
        .uri("/ical/combined-work")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/calendar; charset=utf-8"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();

    // Parse result to verify
    let calendar = parse_calendar(&body_str).unwrap();
    let events = calendar.events();

    // Should have 4 events:
    // - 2 from work calendar (Team standup with modifier, Project review)
    // - 2 from holidays (both holidays)
    // Optional lunch should be filtered out
    assert_eq!(events.len(), 4);

    // Check that Meeting: was replaced with [WORK]
    let team_standup = events.iter().find(|e| {
        e.summary()
            .map(|s| s.contains("Team standup"))
            .unwrap_or(false)
    });
    assert!(team_standup.is_some());
    assert_eq!(team_standup.unwrap().summary(), Some("[WORK] Team standup"));

    // Check that optional lunch is NOT present
    let optional_lunch = events
        .iter()
        .find(|e| e.summary().map(|s| s.contains("Optional")).unwrap_or(false));
    assert!(optional_lunch.is_none());

    // Check holidays are present
    let christmas = events.iter().find(|e| e.summary() == Some("Christmas Day"));
    assert!(christmas.is_some());
}

#[tokio::test]
async fn test_filter_behavior_end_to_end() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/work.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(WORK_CALENDAR))
        .mount(&mock_server)
        .await;

    // Test 1: Only allow step
    let mut calendars = HashMap::new();
    calendars.insert(
        "test".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: format!("{}/work.ics", mock_server.uri()),
                steps: vec![Step::Allow {
                    patterns: vec!["(?i)meeting".to_string()],
                    mode: MatchMode::Any,
                    fields: vec!["summary".to_string(), "description".to_string()],
                }],
            }],
            steps: vec![],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let fetcher = Fetcher::new().unwrap();
    let result = merge_calendars("test", &config, &fetcher).await.unwrap();

    // Should have 2 events that contain "meeting" (Team standup and Project review)
    // "Optional: Lunch and learn" doesn't contain "meeting"
    assert_eq!(result.events.len(), 2);
    assert_eq!(result.errors.len(), 0);

    // Test 2: Only deny step
    let mut calendars = HashMap::new();
    calendars.insert(
        "test".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: format!("{}/work.ics", mock_server.uri()),
                steps: vec![Step::Deny {
                    patterns: vec!["(?i)optional".to_string()],
                    mode: MatchMode::Any,
                    fields: vec!["summary".to_string()],
                }],
            }],
            steps: vec![],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let result = merge_calendars("test", &config, &fetcher).await.unwrap();

    // Should have 2 events (3 total - 1 optional)
    assert_eq!(result.events.len(), 2);
}

#[tokio::test]
async fn test_multiple_sources_with_per_source_filters_and_modifiers() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/work.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(WORK_CALENDAR))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/personal.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PERSONAL_CALENDAR))
        .mount(&mock_server)
        .await;

    let mut calendars = HashMap::new();
    calendars.insert(
        "test".to_string(),
        CalendarConfig {
            sources: vec![
                SourceConfig::Url {
                    url: format!("{}/work.ics", mock_server.uri()),
                    steps: vec![
                        Step::Allow {
                            patterns: vec!["(?i)meeting".to_string()],
                            mode: MatchMode::Any,
                            fields: vec!["summary".to_string()],
                        },
                        Step::Replace {
                            pattern: "Meeting:".to_string(),
                            replacement: "[WORK]".to_string(),
                            field: "summary".to_string(),
                        },
                    ],
                },
                SourceConfig::Url {
                    url: format!("{}/personal.ics", mock_server.uri()),
                    steps: vec![Step::Replace {
                        pattern: "^".to_string(),
                        replacement: "[PERSONAL] ".to_string(),
                        field: "summary".to_string(),
                    }],
                },
            ],
            steps: vec![],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let fetcher = Fetcher::new().unwrap();
    let result = merge_calendars("test", &config, &fetcher).await.unwrap();

    // Work: 2 meetings allowed (Team standup and Project review)
    // Personal: 2 events, both included
    assert_eq!(result.events.len(), 4);

    // Check work modifier
    let work_event = result.events.iter().find(|e| {
        e.summary()
            .map(|s| s.contains("Team standup"))
            .unwrap_or(false)
    });
    assert_eq!(work_event.unwrap().summary(), Some("[WORK] Team standup"));

    // Check personal modifier
    let personal_event = result
        .events
        .iter()
        .find(|e| e.summary().map(|s| s.contains("Dinner")).unwrap_or(false));
    assert_eq!(
        personal_event.unwrap().summary(),
        Some("[PERSONAL] Dinner with friends")
    );
}

#[tokio::test]
async fn test_calendar_level_steps() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/work.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(WORK_CALENDAR))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/personal.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PERSONAL_CALENDAR))
        .mount(&mock_server)
        .await;

    // Both sources have no filtering, but calendar level applies a global prefix
    let mut calendars = HashMap::new();
    calendars.insert(
        "test".to_string(),
        CalendarConfig {
            sources: vec![
                SourceConfig::Url {
                    url: format!("{}/work.ics", mock_server.uri()),
                    steps: vec![],
                },
                SourceConfig::Url {
                    url: format!("{}/personal.ics", mock_server.uri()),
                    steps: vec![],
                },
            ],
            steps: vec![Step::Replace {
                pattern: "^".to_string(),
                replacement: "[MERGED] ".to_string(),
                field: "summary".to_string(),
            }],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let fetcher = Fetcher::new().unwrap();
    let result = merge_calendars("test", &config, &fetcher).await.unwrap();

    // All 5 events should have the prefix
    assert_eq!(result.events.len(), 5);
    for event in &result.events {
        assert!(
            event
                .summary()
                .map(|s| s.starts_with("[MERGED] "))
                .unwrap_or(false)
        );
    }
}

#[tokio::test]
async fn test_match_mode_all() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/work.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(WORK_CALENDAR))
        .mount(&mock_server)
        .await;

    // Allow only events that match ALL patterns
    let mut calendars = HashMap::new();
    calendars.insert(
        "test".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: format!("{}/work.ics", mock_server.uri()),
                steps: vec![Step::Allow {
                    patterns: vec!["(?i)meeting".to_string(), "(?i)team".to_string()],
                    mode: MatchMode::All,
                    fields: vec!["summary".to_string()],
                }],
            }],
            steps: vec![],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let fetcher = Fetcher::new().unwrap();
    let result = merge_calendars("test", &config, &fetcher).await.unwrap();

    // Only "Meeting: Team standup" matches both "meeting" and "team"
    assert_eq!(result.events.len(), 1);
    assert!(
        result.events[0]
            .summary()
            .map(|s| s.contains("Team standup"))
            .unwrap_or(false)
    );
}

#[tokio::test]
async fn test_step_ordering_matters() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/work.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(WORK_CALENDAR))
        .mount(&mock_server)
        .await;

    // Replace then allow - the allow step sees the replaced text
    let mut calendars = HashMap::new();
    calendars.insert(
        "test".to_string(),
        CalendarConfig {
            sources: vec![SourceConfig::Url {
                url: format!("{}/work.ics", mock_server.uri()),
                steps: vec![
                    Step::Replace {
                        pattern: "(?i)meeting".to_string(),
                        replacement: "Event".to_string(),
                        field: "summary".to_string(),
                    },
                    Step::Allow {
                        patterns: vec!["Event".to_string()],
                        mode: MatchMode::Any,
                        fields: vec!["summary".to_string()],
                    },
                ],
            }],
            steps: vec![],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let fetcher = Fetcher::new().unwrap();
    let result = merge_calendars("test", &config, &fetcher).await.unwrap();

    // Only events containing "meeting" (now "Event") should pass
    assert_eq!(result.events.len(), 2);
    for event in &result.events {
        assert!(
            event
                .summary()
                .map(|s| s.contains("Event"))
                .unwrap_or(false)
        );
    }
}
