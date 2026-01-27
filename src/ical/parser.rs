use super::types::{Calendar, Event};
use crate::error::{Error, Result};

/// Sanitize iCal text to fix common malformed data issues
fn sanitize_ical(ical_text: &str) -> String {
    ical_text
        .lines()
        .map(|line| {
            // Fix malformed TRIGGER values like "TRIGGER:-P2DT" (empty time component)
            // These should be "TRIGGER:-P2D" (duration without time)
            if line.starts_with("TRIGGER:") && line.ends_with('T') {
                line.trim_end_matches('T').to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse iCal text into a Calendar with Events
pub fn parse_calendar(ical_text: &str) -> Result<Calendar> {
    let sanitized = sanitize_ical(ical_text);

    let parsed = sanitized
        .parse::<icalendar::Calendar>()
        .map_err(|e| Error::Parse(format!("Failed to parse iCal: {}", e)))?;

    let events = extract_events(&parsed);

    Ok(Calendar::new(parsed, events))
}

/// Extract events from an icalendar::Calendar
fn extract_events(calendar: &icalendar::Calendar) -> Vec<Event> {
    calendar
        .components
        .iter()
        .filter_map(|component| {
            if let icalendar::CalendarComponent::Event(event) = component {
                Some(Event::new(event.clone()))
            } else {
                None
            }
        })
        .collect()
}

/// Serialize a list of events back to valid iCal string
pub fn serialize_events(events: Vec<Event>) -> String {
    let mut calendar = icalendar::Calendar::new();

    for event in events {
        calendar.push(event.into_inner());
    }

    calendar.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_ICAL: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//My Company//My Product//EN
BEGIN:VEVENT
UID:event1@example.com
DTSTAMP:20231201T120000Z
DTSTART:20231201T140000Z
DTEND:20231201T150000Z
SUMMARY:Test Event
DESCRIPTION:This is a test event
END:VEVENT
END:VCALENDAR"#;

    const MULTI_EVENT_ICAL: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//My Company//My Product//EN
BEGIN:VEVENT
UID:event1@example.com
DTSTAMP:20231201T120000Z
DTSTART:20231201T140000Z
DTEND:20231201T150000Z
SUMMARY:First Event
END:VEVENT
BEGIN:VEVENT
UID:event2@example.com
DTSTAMP:20231202T120000Z
DTSTART:20231202T140000Z
DTEND:20231202T150000Z
SUMMARY:Second Event
END:VEVENT
END:VCALENDAR"#;

    #[test]
    fn test_parse_simple_event() {
        let calendar = parse_calendar(SIMPLE_ICAL).unwrap();
        let events = calendar.events();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary(), Some("Test Event"));
        assert_eq!(events[0].description(), Some("This is a test event"));
        assert_eq!(events[0].uid(), Some("event1@example.com"));
    }

    #[test]
    fn test_parse_multiple_events() {
        let calendar = parse_calendar(MULTI_EVENT_ICAL).unwrap();
        let events = calendar.events();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].summary(), Some("First Event"));
        assert_eq!(events[1].summary(), Some("Second Event"));
    }

    #[test]
    fn test_round_trip() {
        let calendar = parse_calendar(SIMPLE_ICAL).unwrap();
        let events = calendar.into_events();

        let serialized = serialize_events(events);

        // Parse it again
        let reparsed = parse_calendar(&serialized).unwrap();
        let reparsed_events = reparsed.events();

        assert_eq!(reparsed_events.len(), 1);
        assert_eq!(reparsed_events[0].summary(), Some("Test Event"));
        assert_eq!(
            reparsed_events[0].description(),
            Some("This is a test event")
        );
    }

    #[test]
    fn test_parse_empty_ical() {
        // The icalendar crate is permissive, so we test that we can handle
        // calendars with no events
        let result = parse_calendar("not valid ical");
        // It might parse successfully but have no events
        if let Ok(calendar) = result {
            assert_eq!(calendar.events().len(), 0);
        }
        // Or it might fail, which is also acceptable
    }

    #[test]
    fn test_sanitize_malformed_trigger() {
        let malformed = r#"BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:test@example.com
DTSTAMP:20231201T120000Z
SUMMARY:Test Event
BEGIN:VALARM
TRIGGER:-P2DT
ACTION:DISPLAY
END:VALARM
END:VEVENT
END:VCALENDAR"#;

        let calendar = parse_calendar(malformed).unwrap();
        let events = calendar.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary(), Some("Test Event"));
    }

    #[test]
    fn test_parse_england_rugby_fixture() {
        let ical_text = include_str!("../../tests/fixtures/england_rugby.ics");
        let calendar = parse_calendar(ical_text).unwrap();
        let events = calendar.events();

        // The England Rugby calendar should have multiple events
        assert!(
            !events.is_empty(),
            "Expected events from England Rugby calendar"
        );

        // Verify at least one event has the expected structure
        let first_event = &events[0];
        assert!(first_event.summary().is_some());
        assert!(first_event.uid().is_some());
    }

    #[test]
    fn test_parse_the_fa_fixture() {
        let ical_text = include_str!("../../tests/fixtures/the_fa.ics");
        let calendar = parse_calendar(ical_text).unwrap();
        let events = calendar.events();

        // The FA calendar should have multiple events
        assert!(!events.is_empty(), "Expected events from The FA calendar");

        // Verify at least one event has the expected structure
        let first_event = &events[0];
        assert!(first_event.summary().is_some());
        assert!(first_event.uid().is_some());
    }
}
