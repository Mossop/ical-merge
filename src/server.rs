use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::config::Config;
use crate::fetcher::Fetcher;
use crate::ical::parser::serialize_events;
use crate::merge::merge_calendars;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub fetcher: Arc<Fetcher>,
}

impl AppState {
    pub fn new(config: Config, fetcher: Fetcher) -> Self {
        Self {
            config: Arc::new(config),
            fetcher: Arc::new(fetcher),
        }
    }
}

/// Create the router with all routes
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/ical/{id}", get(get_calendar))
        .with_state(state)
}

/// Handler for GET /ical/{id}
async fn get_calendar(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    // Look up calendar config
    let calendar_config = state
        .config
        .calendars
        .get(&id)
        .ok_or_else(|| AppError::NotFound(format!("Calendar '{}' not found", id)))?;

    // Merge calendars
    let merge_result = merge_calendars(calendar_config, &state.fetcher).await?;

    // Log any errors but still serve partial data
    for (url, err) in &merge_result.errors {
        tracing::error!("Failed to fetch calendar from {}: {}", url, err);
    }

    // Serialize to iCal format
    let ical_text = serialize_events(merge_result.events);

    // Return with proper content type
    Ok((
        [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
        ical_text,
    )
        .into_response())
}

/// Application error type
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Internal(crate::error::Error),
}

impl From<crate::error::Error> for AppError {
    fn from(err: crate::error::Error) -> Self {
        AppError::Internal(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Internal(err) => {
                tracing::error!("Internal error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        (status, message).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CalendarConfig, ServerConfig, SourceConfig};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::collections::HashMap;
    use tower::util::ServiceExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SAMPLE_ICAL: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:test@example.com
DTSTAMP:20231201T120000Z
DTSTART:20231201T140000Z
DTEND:20231201T150000Z
SUMMARY:Test Event
END:VEVENT
END:VCALENDAR"#;

    #[tokio::test]
    async fn test_get_calendar_endpoint() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/test.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ICAL))
            .mount(&mock_server)
            .await;

        let mut calendars = HashMap::new();
        calendars.insert(
            "test-calendar".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
                    url: format!("{}/test.ics", mock_server.uri()),
                    filters: Default::default(),
                    modifiers: vec![],
                }],
            },
        );

        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };

        let fetcher = Fetcher::new().unwrap();
        let state = AppState::new(config, fetcher);
        let app = create_router(state);

        let request = Request::builder()
            .uri("/ical/test-calendar")
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
        assert!(body_str.contains("Test Event"));
    }

    #[tokio::test]
    async fn test_unknown_calendar_returns_404() {
        let config = Config {
            server: ServerConfig::default(),
            calendars: HashMap::new(),
        };

        let fetcher = Fetcher::new().unwrap();
        let state = AppState::new(config, fetcher);
        let app = create_router(state);

        let request = Request::builder()
            .uri("/ical/nonexistent")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_partial_failure_still_serves() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/test.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ICAL))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/notfound.ics"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let mut calendars = HashMap::new();
        calendars.insert(
            "test-calendar".to_string(),
            CalendarConfig {
                sources: vec![
                    SourceConfig {
                        url: format!("{}/test.ics", mock_server.uri()),
                        filters: Default::default(),
                        modifiers: vec![],
                    },
                    SourceConfig {
                        url: format!("{}/notfound.ics", mock_server.uri()),
                        filters: Default::default(),
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

        let request = Request::builder()
            .uri("/ical/test-calendar")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Should still succeed with partial data
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Test Event"));
    }
}
