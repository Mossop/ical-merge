use futures::future::join_all;

use crate::config::{CalendarConfig, SourceConfig};
use crate::error::{Error, Result};
use crate::fetcher::Fetcher;
use crate::filter::{CompiledFilter, CompiledModifier};
use crate::ical::{Event, parse_calendar};

/// Result of merging multiple calendar sources
#[derive(Debug)]
pub struct MergeResult {
    pub events: Vec<Event>,
    pub errors: Vec<(String, Error)>,
}

impl MergeResult {
    pub fn new(events: Vec<Event>, errors: Vec<(String, Error)>) -> Self {
        Self { events, errors }
    }
}

/// Fetch and merge calendars according to config
pub async fn merge_calendars(config: &CalendarConfig, fetcher: &Fetcher) -> Result<MergeResult> {
    let futures: Vec<_> = config
        .sources
        .iter()
        .map(|source| fetch_and_process_source(source, fetcher))
        .collect();

    let results = join_all(futures).await;

    let mut all_events = Vec::new();
    let mut errors = Vec::new();

    for result in results {
        match result {
            Ok(events) => all_events.extend(events),
            Err((url, err)) => errors.push((url, err)),
        }
    }

    Ok(MergeResult::new(all_events, errors))
}

/// Fetch and process a single source
async fn fetch_and_process_source(
    source: &SourceConfig,
    fetcher: &Fetcher,
) -> std::result::Result<Vec<Event>, (String, Error)> {
    let url = source.url.clone();

    // Fetch calendar
    let ical_text = fetcher.fetch(&url).await.map_err(|e| (url.clone(), e))?;

    // Parse calendar
    let calendar = parse_calendar(&ical_text).map_err(|e| (url.clone(), e))?;

    // Compile filter
    let filter = CompiledFilter::compile(&source.filters).map_err(|e| (url.clone(), e))?;

    // Compile modifiers
    let modifiers =
        CompiledModifier::compile_many(&source.modifiers).map_err(|e| (url.clone(), e))?;

    // Filter events
    let filtered_events: Vec<Event> = calendar
        .into_events()
        .into_iter()
        .filter(|event| filter.should_include(event))
        .collect();

    // Apply modifiers
    let mut modified_events = Vec::new();
    for mut event in filtered_events {
        for modifier in &modifiers {
            modifier.apply(&mut event);
        }
        modified_events.push(event);
    }

    Ok(modified_events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FilterConfig, FilterRule, ModifierConfig, SourceConfig};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const CALENDAR1: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:event1@example.com
DTSTAMP:20231201T120000Z
DTSTART:20231201T140000Z
DTEND:20231201T150000Z
SUMMARY:Meeting with team
END:VEVENT
BEGIN:VEVENT
UID:event2@example.com
DTSTAMP:20231202T120000Z
DTSTART:20231202T140000Z
DTEND:20231202T150000Z
SUMMARY:Optional lunch
END:VEVENT
END:VCALENDAR"#;

    const CALENDAR2: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:event3@example.com
DTSTAMP:20231203T120000Z
DTSTART:20231203T140000Z
DTEND:20231203T150000Z
SUMMARY:Holiday
END:VEVENT
END:VCALENDAR"#;

    #[tokio::test]
    async fn test_merge_multiple_calendars() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/cal1.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR1))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/cal2.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR2))
            .mount(&mock_server)
            .await;

        let config = CalendarConfig {
            sources: vec![
                SourceConfig {
                    url: format!("{}/cal1.ics", mock_server.uri()),
                    filters: FilterConfig::default(),
                    modifiers: vec![],
                },
                SourceConfig {
                    url: format!("{}/cal2.ics", mock_server.uri()),
                    filters: FilterConfig::default(),
                    modifiers: vec![],
                },
            ],
        };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars(&config, &fetcher).await.unwrap();

        assert_eq!(result.events.len(), 3);
        assert_eq!(result.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_merge_with_per_source_filters() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/cal1.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR1))
            .mount(&mock_server)
            .await;

        let config = CalendarConfig {
            sources: vec![SourceConfig {
                url: format!("{}/cal1.ics", mock_server.uri()),
                filters: FilterConfig {
                    allow: vec![FilterRule {
                        pattern: "(?i)meeting".to_string(),
                        fields: vec!["summary".to_string()],
                    }],
                    deny: vec![],
                },
                modifiers: vec![],
            }],
        };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars(&config, &fetcher).await.unwrap();

        // Only "Meeting with team" should be included
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].summary(), Some("Meeting with team"));
        assert_eq!(result.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_merge_with_modifiers() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/cal1.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR1))
            .mount(&mock_server)
            .await;

        let config = CalendarConfig {
            sources: vec![SourceConfig {
                url: format!("{}/cal1.ics", mock_server.uri()),
                filters: FilterConfig {
                    allow: vec![FilterRule {
                        pattern: "(?i)meeting".to_string(),
                        fields: vec!["summary".to_string()],
                    }],
                    deny: vec![],
                },
                modifiers: vec![ModifierConfig::Replace {
                    pattern: "^Meeting".to_string(),
                    replacement: "[WORK]".to_string(),
                    field: "summary".to_string(),
                }],
            }],
        };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars(&config, &fetcher).await.unwrap();

        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].summary(), Some("[WORK] with team"));
        assert_eq!(result.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_partial_failure() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/cal1.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR1))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/notfound.ics"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let config = CalendarConfig {
            sources: vec![
                SourceConfig {
                    url: format!("{}/cal1.ics", mock_server.uri()),
                    filters: FilterConfig::default(),
                    modifiers: vec![],
                },
                SourceConfig {
                    url: format!("{}/notfound.ics", mock_server.uri()),
                    filters: FilterConfig::default(),
                    modifiers: vec![],
                },
            ],
        };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars(&config, &fetcher).await.unwrap();

        // Should have events from cal1 but error for cal2
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].0.contains("notfound.ics"));
    }
}
