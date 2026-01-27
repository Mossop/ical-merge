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
}
