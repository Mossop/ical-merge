use std::collections::HashSet;

use futures::future::join_all;

use crate::config::{Config, SourceConfig};
use crate::error::{Error, Result};
use crate::fetcher::Fetcher;
use crate::filter::{CompiledStep, process_events};
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

/// Type alias for event time boundaries
type EventTimeBoundary = (Option<i64>, Option<i64>);

/// Convert DatePerhapsTime to timestamp for comparison
fn date_to_timestamp(dpt: &icalendar::DatePerhapsTime) -> i64 {
    use icalendar::DatePerhapsTime;

    match dpt {
        DatePerhapsTime::DateTime(dt) => match dt {
            icalendar::CalendarDateTime::Floating(naive) => naive.and_utc().timestamp(),
            icalendar::CalendarDateTime::Utc(utc) => utc.timestamp(),
            icalendar::CalendarDateTime::WithTimezone { date_time, .. } => {
                date_time.and_utc().timestamp()
            }
        },
        DatePerhapsTime::Date(date) => date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp(),
    }
}

/// Extract time boundary (start, end) from an event as timestamps
fn extract_time_boundary(event: &Event) -> EventTimeBoundary {
    let start = event.start().map(|dt| date_to_timestamp(&dt));
    let end = event.end().map(|dt| date_to_timestamp(&dt));
    (start, end)
}

/// Deduplicate events by (start, end) time, keeping only the first occurrence
fn deduplicate_events(events: Vec<Event>) -> Vec<Event> {
    let mut seen = HashSet::new();
    let mut deduplicated = Vec::new();

    for event in events {
        let time_boundary = extract_time_boundary(&event);

        if seen.insert(time_boundary) {
            deduplicated.push(event);
        }
    }

    deduplicated
}

/// Fetch and merge calendars according to config
pub async fn merge_calendars(
    calendar_id: &str,
    config: &Config,
    fetcher: &Fetcher,
) -> Result<MergeResult> {
    let calendar_config = config
        .calendars
        .get(calendar_id)
        .ok_or_else(|| Error::Config(format!("Calendar '{}' not found", calendar_id)))?;

    let futures: Vec<_> = calendar_config
        .sources
        .iter()
        .map(|source| fetch_and_process_source(source, config, fetcher))
        .collect();

    let results = join_all(futures).await;

    let mut all_events = Vec::new();
    let mut errors = Vec::new();

    for result in results {
        match result {
            Ok(events) => all_events.extend(events),
            Err((identifier, err)) => errors.push((identifier, err)),
        }
    }

    // Apply calendar-level steps
    let calendar_steps = CompiledStep::compile_many(&calendar_config.steps)
        .map_err(|e| Error::Config(format!("Failed to compile calendar-level steps: {}", e)))?;
    let processed_events = process_events(all_events, &calendar_steps);

    // Deduplicate events by (start, end) time
    let deduplicated_events = deduplicate_events(processed_events);

    Ok(MergeResult::new(deduplicated_events, errors))
}

/// Fetch and process a single source
async fn fetch_and_process_source(
    source: &SourceConfig,
    config: &Config,
    fetcher: &Fetcher,
) -> std::result::Result<Vec<Event>, (String, Error)> {
    let identifier = source.identifier();

    // Get events from either URL or calendar reference
    let events = match source {
        SourceConfig::Url { url, .. } => {
            // Fetch calendar
            let ical_text = fetcher
                .fetch(url)
                .await
                .map_err(|e| (identifier.clone(), e))?;

            // Parse calendar
            let calendar = parse_calendar(&ical_text).map_err(|e| (identifier.clone(), e))?;

            calendar.into_events()
        }
        SourceConfig::Calendar {
            calendar: ref_id, ..
        } => {
            // Resolve calendar reference
            resolve_calendar_reference(ref_id, config, fetcher)
                .await
                .map_err(|e| (identifier.clone(), e))?
        }
    };

    // Compile and apply source-level steps
    let steps = CompiledStep::compile_many(source.steps()).map_err(|e| (identifier.clone(), e))?;
    let processed_events = process_events(events, &steps);

    Ok(processed_events)
}

/// Resolve a calendar reference by recursively calling merge_calendars
async fn resolve_calendar_reference(
    calendar_id: &str,
    config: &Config,
    fetcher: &Fetcher,
) -> Result<Vec<Event>> {
    let merge_result = merge_calendars(calendar_id, config, fetcher).await?;

    // Log errors from referenced calendar
    for (identifier, err) in &merge_result.errors {
        tracing::error!(
            "Error in referenced calendar '{}' from {}: {}",
            calendar_id,
            identifier,
            err
        );
    }

    Ok(merge_result.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CalendarConfig, MatchMode, SourceConfig, Step};
    use std::collections::HashMap;
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

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![
                    SourceConfig::Url {
                        url: format!("{}/cal1.ics", mock_server.uri()),
                        steps: vec![],
                    },
                    SourceConfig::Url {
                        url: format!("{}/cal2.ics", mock_server.uri()),
                        steps: vec![],
                    },
                ],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("test", &config, &fetcher).await.unwrap();

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

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: format!("{}/cal1.ics", mock_server.uri()),
                    steps: vec![Step::Allow {
                        patterns: vec!["(?i)meeting".to_string()],
                        mode: MatchMode::Any,
                        fields: vec!["summary".to_string()],
                    }],
                }],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("test", &config, &fetcher).await.unwrap();

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

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: format!("{}/cal1.ics", mock_server.uri()),
                    steps: vec![
                        Step::Allow {
                            patterns: vec!["(?i)meeting".to_string()],
                            mode: MatchMode::Any,
                            fields: vec!["summary".to_string()],
                        },
                        Step::Replace {
                            pattern: "^Meeting".to_string(),
                            replacement: "[WORK]".to_string(),
                            field: "summary".to_string(),
                        },
                    ],
                }],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("test", &config, &fetcher).await.unwrap();

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

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![
                    SourceConfig::Url {
                        url: format!("{}/cal1.ics", mock_server.uri()),
                        steps: vec![],
                    },
                    SourceConfig::Url {
                        url: format!("{}/notfound.ics", mock_server.uri()),
                        steps: vec![],
                    },
                ],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("test", &config, &fetcher).await.unwrap();

        // Should have events from cal1 but error for cal2
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].0.contains("notfound.ics"));
    }

    #[tokio::test]
    async fn test_deduplication_by_time() {
        let mock_server = MockServer::start().await;

        // Two calendars with overlapping events (same start/end times)
        const CAL_WITH_DUP1: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:event1@example.com
DTSTAMP:20231201T120000Z
DTSTART:20231201T140000Z
DTEND:20231201T150000Z
SUMMARY:Meeting from Calendar 1
END:VEVENT
BEGIN:VEVENT
UID:event2@example.com
DTSTAMP:20231202T120000Z
DTSTART:20231202T140000Z
DTEND:20231202T150000Z
SUMMARY:Unique Event 1
END:VEVENT
END:VCALENDAR"#;

        const CAL_WITH_DUP2: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:different-uid@example.com
DTSTAMP:20231201T120000Z
DTSTART:20231201T140000Z
DTEND:20231201T150000Z
SUMMARY:Meeting from Calendar 2
DESCRIPTION:This is a duplicate time slot
END:VEVENT
BEGIN:VEVENT
UID:event3@example.com
DTSTAMP:20231203T120000Z
DTSTART:20231203T140000Z
DTEND:20231203T150000Z
SUMMARY:Unique Event 2
END:VEVENT
END:VCALENDAR"#;

        Mock::given(method("GET"))
            .and(path("/cal1.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CAL_WITH_DUP1))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/cal2.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CAL_WITH_DUP2))
            .mount(&mock_server)
            .await;

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![
                    SourceConfig::Url {
                        url: format!("{}/cal1.ics", mock_server.uri()),
                        steps: vec![],
                    },
                    SourceConfig::Url {
                        url: format!("{}/cal2.ics", mock_server.uri()),
                        steps: vec![],
                    },
                ],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("test", &config, &fetcher).await.unwrap();

        // Should have 3 events: 2 from cal1, 1 from cal2 (duplicate removed)
        assert_eq!(result.events.len(), 3);
        assert_eq!(result.errors.len(), 0);

        // First event with 2023-12-01 14:00-15:00 should be from Calendar 1
        let first_meeting = result
            .events
            .iter()
            .find(|e| e.summary() == Some("Meeting from Calendar 1"));
        assert!(
            first_meeting.is_some(),
            "First occurrence should be kept (from Calendar 1)"
        );

        // Second occurrence from Calendar 2 should be filtered out
        let second_meeting = result
            .events
            .iter()
            .find(|e| e.summary() == Some("Meeting from Calendar 2"));
        assert!(
            second_meeting.is_none(),
            "Duplicate from Calendar 2 should be removed"
        );

        // Both unique events should be present
        assert!(
            result
                .events
                .iter()
                .any(|e| e.summary() == Some("Unique Event 1"))
        );
        assert!(
            result
                .events
                .iter()
                .any(|e| e.summary() == Some("Unique Event 2"))
        );
    }

    #[tokio::test]
    async fn test_calendar_reference() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/base.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR1))
            .mount(&mock_server)
            .await;

        let mut calendars = HashMap::new();
        calendars.insert(
            "base".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: format!("{}/base.ics", mock_server.uri()),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "derived".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "base".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("derived", &config, &fetcher).await.unwrap();

        // Should have events from base calendar
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_calendar_reference_with_steps() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/base.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR1))
            .mount(&mock_server)
            .await;

        let mut calendars = HashMap::new();
        calendars.insert(
            "base".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: format!("{}/base.ics", mock_server.uri()),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "derived".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "base".to_string(),
                    steps: vec![Step::Replace {
                        pattern: "^".to_string(),
                        replacement: "[WORK] ".to_string(),
                        field: "summary".to_string(),
                    }],
                }],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("derived", &config, &fetcher).await.unwrap();

        // Should have events from base calendar with source-level steps applied
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].summary(), Some("[WORK] Meeting with team"));
        assert_eq!(result.events[1].summary(), Some("[WORK] Optional lunch"));
        assert_eq!(result.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_mixed_url_and_calendar_sources() {
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

        let mut calendars = HashMap::new();
        calendars.insert(
            "base".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: format!("{}/cal1.ics", mock_server.uri()),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "combined".to_string(),
            CalendarConfig {
                sources: vec![
                    SourceConfig::Calendar {
                        calendar: "base".to_string(),
                        steps: vec![],
                    },
                    SourceConfig::Url {
                        url: format!("{}/cal2.ics", mock_server.uri()),
                        steps: vec![],
                    },
                ],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("combined", &config, &fetcher)
            .await
            .unwrap();

        // Should have events from both sources
        assert_eq!(result.events.len(), 3);
        assert_eq!(result.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_nested_calendar_references() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/base.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(CALENDAR1))
            .mount(&mock_server)
            .await;

        let mut calendars = HashMap::new();
        calendars.insert(
            "level1".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: format!("{}/base.ics", mock_server.uri()),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "level2".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "level1".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "level3".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "level2".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );

        let config = Config { calendars };

        let fetcher = Fetcher::new().unwrap();
        let result = merge_calendars("level3", &config, &fetcher).await.unwrap();

        // Should resolve through all levels
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.errors.len(), 0);
    }
}
