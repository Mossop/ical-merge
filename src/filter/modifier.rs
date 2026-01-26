use regex::Regex;

use crate::config::ModifierConfig;
use crate::error::Result;
use crate::ical::Event;

/// A compiled modifier with regex and replacement
#[derive(Debug)]
pub struct CompiledModifier {
    regex: Regex,
    replacement: String,
}

impl CompiledModifier {
    pub fn new(pattern: &str, replacement: &str) -> Result<Self> {
        let regex = Regex::new(pattern)?;
        Ok(Self {
            regex,
            replacement: replacement.to_string(),
        })
    }

    pub fn compile_many(configs: &[ModifierConfig]) -> Result<Vec<Self>> {
        configs
            .iter()
            .map(|config| Self::new(&config.pattern, &config.replacement))
            .collect()
    }

    /// Apply this modifier to an event's summary
    pub fn apply(&self, event: &mut Event) {
        if let Some(summary) = event.summary().map(|s| s.to_string()) {
            let new_summary = self.regex.replace_all(&summary, &self.replacement);
            event.set_summary(&new_summary);
        }
    }
}

/// Apply all modifiers to an event in order
pub fn apply_modifiers(event: &mut Event, modifiers: &[CompiledModifier]) {
    for modifier in modifiers {
        modifier.apply(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use icalendar::Component;

    fn create_event(summary: &str) -> Event {
        let mut event = icalendar::Event::new();
        event.summary(summary);
        Event::new(event)
    }

    #[test]
    fn test_simple_replacement() {
        let modifier = CompiledModifier::new("^Meeting:", "[WORK] ").unwrap();
        let mut event = create_event("Meeting: Team sync");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("[WORK]  Team sync"));
    }

    #[test]
    fn test_capture_groups() {
        let modifier = CompiledModifier::new(r"^Meeting: (.+)$", "[WORK] $1").unwrap();
        let mut event = create_event("Meeting: Team sync");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("[WORK] Team sync"));
    }

    #[test]
    fn test_no_match_no_change() {
        let modifier = CompiledModifier::new("^Meeting:", "[WORK] ").unwrap();
        let mut event = create_event("Lunch break");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("Lunch break"));
    }

    #[test]
    fn test_modifier_ordering() {
        let modifiers = vec![
            CompiledModifier::new("Meeting", "Event").unwrap(),
            CompiledModifier::new("Event", "Activity").unwrap(),
        ];

        let mut event = create_event("Meeting with team");

        apply_modifiers(&mut event, &modifiers);

        // First modifier changes "Meeting" to "Event"
        // Second modifier changes "Event" to "Activity"
        assert_eq!(event.summary(), Some("Activity with team"));
    }

    #[test]
    fn test_multiple_replacements_in_one_string() {
        let modifier = CompiledModifier::new("meeting", "event").unwrap();
        let mut event = create_event("meeting about meeting preparation");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("event about event preparation"));
    }

    #[test]
    fn test_compile_many() {
        let configs = vec![
            ModifierConfig {
                pattern: "Meeting".to_string(),
                replacement: "Event".to_string(),
            },
            ModifierConfig {
                pattern: "(?i)urgent".to_string(),
                replacement: "[URGENT]".to_string(),
            },
        ];

        let modifiers = CompiledModifier::compile_many(&configs).unwrap();
        assert_eq!(modifiers.len(), 2);

        let mut event = create_event("Urgent Meeting");
        apply_modifiers(&mut event, &modifiers);

        // "Meeting" -> "Event", "Urgent" -> "[URGENT]"
        assert_eq!(event.summary(), Some("[URGENT] Event"));
    }
}
