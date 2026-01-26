use axum::body::Body;
use axum::http::{Request, StatusCode};
use ical_merge::config::{
    CalendarConfig, Config, FilterConfig, FilterRule, ModifierConfig, ServerConfig, SourceConfig,
};
use ical_merge::fetcher::Fetcher;
use ical_merge::ical::parse_calendar;
use ical_merge::merge::merge_calendars;
use ical_merge::server::{create_router, AppState};
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
    // - Filter: allow only meetings, deny optional
    // - Modifier: prefix "Meeting:" with "[WORK]"
    let mut calendars = HashMap::new();
    calendars.insert(
        "combined-work".to_string(),
        CalendarConfig {
            sources: vec![
                SourceConfig {
                    url: format!("{}/work.ics", mock_server.uri()),
                    filters: FilterConfig {
                        allow: vec![FilterRule {
                            pattern: "(?i)meeting".to_string(),
                            fields: vec!["summary".to_string(), "description".to_string()],
                        }],
                        deny: vec![FilterRule {
                            pattern: "(?i)optional".to_string(),
                            fields: vec!["summary".to_string()],
                        }],
                    },
                    modifiers: vec![ModifierConfig {
                        pattern: "^Meeting:".to_string(),
                        replacement: "[WORK]".to_string(),
                    }],
                },
                SourceConfig {
                    url: format!("{}/holidays.ics", mock_server.uri()),
                    filters: FilterConfig::default(),
                    modifiers: vec![],
                },
            ],
        },
    );

    let config = Config {
        server: ServerConfig::default(),
        calendars,
    };

    let fetcher = Fetcher::new().unwrap();
    let state = AppState::new(config, fetcher);
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

    // Test 1: Only allow rules
    let config = CalendarConfig {
        sources: vec![SourceConfig {
            url: format!("{}/work.ics", mock_server.uri()),
            filters: FilterConfig {
                allow: vec![FilterRule {
                    pattern: "(?i)meeting".to_string(),
                    fields: vec!["summary".to_string(), "description".to_string()],
                }],
                deny: vec![],
            },
            modifiers: vec![],
        }],
    };

    let fetcher = Fetcher::new().unwrap();
    let result = merge_calendars(&config, &fetcher).await.unwrap();

    // Should have 2 events that contain "meeting" (Team standup and Project review)
    // "Optional: Lunch and learn" doesn't contain "meeting"
    assert_eq!(result.events.len(), 2);
    assert_eq!(result.errors.len(), 0);

    // Test 2: Only deny rules
    let config = CalendarConfig {
        sources: vec![SourceConfig {
            url: format!("{}/work.ics", mock_server.uri()),
            filters: FilterConfig {
                allow: vec![],
                deny: vec![FilterRule {
                    pattern: "(?i)optional".to_string(),
                    fields: vec!["summary".to_string()],
                }],
            },
            modifiers: vec![],
        }],
    };

    let result = merge_calendars(&config, &fetcher).await.unwrap();

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

    let config = CalendarConfig {
        sources: vec![
            SourceConfig {
                url: format!("{}/work.ics", mock_server.uri()),
                filters: FilterConfig {
                    allow: vec![FilterRule {
                        pattern: "(?i)meeting".to_string(),
                        fields: vec!["summary".to_string()],
                    }],
                    deny: vec![],
                },
                modifiers: vec![ModifierConfig {
                    pattern: "Meeting:".to_string(),
                    replacement: "[WORK]".to_string(),
                }],
            },
            SourceConfig {
                url: format!("{}/personal.ics", mock_server.uri()),
                filters: FilterConfig::default(),
                modifiers: vec![ModifierConfig {
                    pattern: "^".to_string(),
                    replacement: "[PERSONAL] ".to_string(),
                }],
            },
        ],
    };

    let fetcher = Fetcher::new().unwrap();
    let result = merge_calendars(&config, &fetcher).await.unwrap();

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
