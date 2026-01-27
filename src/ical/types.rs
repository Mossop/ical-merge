use std::fmt;

use icalendar::{Component, EventLike};

/// Wrapper around icalendar::Calendar
#[derive(Debug)]
pub struct Calendar {
    inner: icalendar::Calendar,
    events: Vec<Event>,
}

impl Calendar {
    pub fn new(inner: icalendar::Calendar, events: Vec<Event>) -> Self {
        Self { inner, events }
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn into_events(self) -> Vec<Event> {
        self.events
    }

    pub fn inner(&self) -> &icalendar::Calendar {
        &self.inner
    }
}

impl fmt::Display for Calendar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

/// Wrapper around icalendar::Event with convenient accessors
#[derive(Debug, Clone)]
pub struct Event {
    inner: icalendar::Event,
}

impl Event {
    pub fn new(inner: icalendar::Event) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &icalendar::Event {
        &self.inner
    }

    pub fn into_inner(self) -> icalendar::Event {
        self.inner
    }

    pub fn summary(&self) -> Option<&str> {
        self.inner.get_summary()
    }

    pub fn description(&self) -> Option<&str> {
        self.inner.get_description()
    }

    pub fn location(&self) -> Option<&str> {
        self.inner.get_location()
    }

    pub fn uid(&self) -> Option<&str> {
        // icalendar doesn't expose get_uid, so we need to get it from properties
        self.inner
            .properties()
            .iter()
            .find(|(key, _)| key.as_str() == "UID")
            .map(|(_, prop)| prop.value())
    }

    pub fn set_summary(&mut self, summary: &str) {
        self.inner.summary(summary);
    }

    pub fn set_description(&mut self, description: &str) {
        self.inner.description(description);
    }

    pub fn set_location(&mut self, location: &str) {
        self.inner.location(location);
    }

    /// Check if this event has any alarms/reminders
    pub fn has_alarms(&self) -> bool {
        // Check if the event's components include any alarms
        // We do this by checking if the inner event's to_string contains VALARM
        self.inner.to_string().contains("BEGIN:VALARM")
    }

    /// Remove all alarm components from this event
    pub fn strip_alarms(&mut self) {
        // The icalendar crate stores alarms as internal components
        // We need to recreate the event without alarms
        let mut new_event = icalendar::Event::new();

        // Copy all properties except alarms
        for prop in self.inner.properties().values() {
            new_event.append_property(prop.clone());
        }

        // Replace the inner event
        self.inner = new_event;
    }

    pub fn start(&self) -> Option<icalendar::DatePerhapsTime> {
        self.inner.get_start()
    }

    pub fn end(&self) -> Option<icalendar::DatePerhapsTime> {
        self.inner.get_end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_accessors() {
        let mut event = icalendar::Event::new();
        event.summary("Test Event");
        event.description("Test Description");
        event.location("Test Location");
        event.uid("test-uid-123");

        let event = Event::new(event);

        assert_eq!(event.summary(), Some("Test Event"));
        assert_eq!(event.description(), Some("Test Description"));
        assert_eq!(event.location(), Some("Test Location"));
        assert_eq!(event.uid(), Some("test-uid-123"));
    }

    #[test]
    fn test_event_set_summary() {
        let mut event = icalendar::Event::new();
        event.summary("Original");

        let mut event = Event::new(event);
        event.set_summary("Modified");

        assert_eq!(event.summary(), Some("Modified"));
    }

    #[test]
    fn test_event_has_alarms() {
        // Event without alarms
        let mut event = icalendar::Event::new();
        event.summary("Test Event");
        let event = Event::new(event);
        assert!(!event.has_alarms());
    }

    #[test]
    fn test_event_has_alarms_after_strip() {
        // Note: We can't easily create an event with alarms in tests without
        // parsing an actual iCal file, so we test with fixture files
        let ical_text = include_str!("../../tests/fixtures/england_rugby.ics");
        let calendar = crate::ical::parse_calendar(ical_text).unwrap();
        let events = calendar.events();

        // England Rugby fixture has alarms
        assert!(events[0].has_alarms());

        // After stripping, should have no alarms
        let mut event_copy = events[0].clone();
        event_copy.strip_alarms();
        assert!(!event_copy.has_alarms());
    }
}
