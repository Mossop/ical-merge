use regex::Regex;

use crate::config::ModifierConfig;
use crate::error::Result;
use crate::ical::Event;

/// A compiled modifier that can perform different types of modifications
#[derive(Debug)]
pub enum CompiledModifier {
    Replace {
        regex: Regex,
        replacement: String,
        field: String,
    },
    StripReminders,
}

impl CompiledModifier {
    pub fn new_replace(pattern: &str, replacement: &str, field: &str) -> Result<Self> {
        let regex = Regex::new(pattern)?;
        Ok(Self::Replace {
            regex,
            replacement: replacement.to_string(),
            field: field.to_string(),
        })
    }

    pub fn compile_many(configs: &[ModifierConfig]) -> Result<Vec<Self>> {
        configs
            .iter()
            .map(|config| match config {
                ModifierConfig::Replace {
                    pattern,
                    replacement,
                    field,
                } => Self::new_replace(pattern, replacement, field),
                ModifierConfig::StripReminders => Ok(Self::StripReminders),
            })
            .collect()
    }

    /// Apply this modifier to an event
    pub fn apply(&self, event: &mut Event) {
        match self {
            Self::Replace {
                regex,
                replacement,
                field,
            } => {
                let text = match field.as_str() {
                    "summary" => event.summary().map(|s| s.to_string()),
                    "description" => event.description().map(|s| s.to_string()),
                    "location" => event.location().map(|s| s.to_string()),
                    _ => None,
                };

                if let Some(text) = text {
                    let new_text = regex.replace_all(&text, replacement);
                    match field.as_str() {
                        "summary" => event.set_summary(&new_text),
                        "description" => event.set_description(&new_text),
                        "location" => event.set_location(&new_text),
                        _ => {}
                    }
                }
            }
            Self::StripReminders => {
                event.strip_alarms();
            }
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
    use icalendar::{Component, EventLike};

    fn create_event(summary: &str) -> Event {
        let mut event = icalendar::Event::new();
        event.summary(summary);
        Event::new(event)
    }

    fn create_event_full(summary: &str, description: &str, location: &str) -> Event {
        let mut event = icalendar::Event::new();
        event.summary(summary);
        event.description(description);
        event.location(location);
        Event::new(event)
    }

    #[test]
    fn test_simple_replacement() {
        let modifier = CompiledModifier::new_replace("^Meeting:", "[WORK] ", "summary").unwrap();
        let mut event = create_event("Meeting: Team sync");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("[WORK]  Team sync"));
    }

    #[test]
    fn test_capture_groups() {
        let modifier =
            CompiledModifier::new_replace(r"^Meeting: (.+)$", "[WORK] $1", "summary").unwrap();
        let mut event = create_event("Meeting: Team sync");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("[WORK] Team sync"));
    }

    #[test]
    fn test_no_match_no_change() {
        let modifier = CompiledModifier::new_replace("^Meeting:", "[WORK] ", "summary").unwrap();
        let mut event = create_event("Lunch break");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("Lunch break"));
    }

    #[test]
    fn test_modifier_ordering() {
        let modifiers = vec![
            CompiledModifier::new_replace("Meeting", "Event", "summary").unwrap(),
            CompiledModifier::new_replace("Event", "Activity", "summary").unwrap(),
        ];

        let mut event = create_event("Meeting with team");

        apply_modifiers(&mut event, &modifiers);

        // First modifier changes "Meeting" to "Event"
        // Second modifier changes "Event" to "Activity"
        assert_eq!(event.summary(), Some("Activity with team"));
    }

    #[test]
    fn test_multiple_replacements_in_one_string() {
        let modifier = CompiledModifier::new_replace("meeting", "event", "summary").unwrap();
        let mut event = create_event("meeting about meeting preparation");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("event about event preparation"));
    }

    #[test]
    fn test_compile_many() {
        let configs = vec![
            ModifierConfig::Replace {
                pattern: "Meeting".to_string(),
                replacement: "Event".to_string(),
                field: "summary".to_string(),
            },
            ModifierConfig::Replace {
                pattern: "(?i)urgent".to_string(),
                replacement: "[URGENT]".to_string(),
                field: "summary".to_string(),
            },
        ];

        let modifiers = CompiledModifier::compile_many(&configs).unwrap();
        assert_eq!(modifiers.len(), 2);

        let mut event = create_event("Urgent Meeting");
        apply_modifiers(&mut event, &modifiers);

        // "Meeting" -> "Event", "Urgent" -> "[URGENT]"
        assert_eq!(event.summary(), Some("[URGENT] Event"));
    }

    #[test]
    fn test_replace_description() {
        let modifier =
            CompiledModifier::new_replace("meeting", "discussion", "description").unwrap();
        let mut event = create_event_full("Event", "Let's have a meeting", "Office");

        modifier.apply(&mut event);

        assert_eq!(event.description(), Some("Let's have a discussion"));
        assert_eq!(event.summary(), Some("Event")); // Unchanged
    }

    #[test]
    fn test_replace_location() {
        let modifier =
            CompiledModifier::new_replace("Office", "Conference Room A", "location").unwrap();
        let mut event = create_event_full("Meeting", "Team sync", "Office");

        modifier.apply(&mut event);

        assert_eq!(event.location(), Some("Conference Room A"));
        assert_eq!(event.summary(), Some("Meeting")); // Unchanged
    }

    #[test]
    fn test_strip_reminders() {
        let modifier = CompiledModifier::StripReminders;
        let mut event = create_event("Meeting");

        modifier.apply(&mut event);

        assert_eq!(event.summary(), Some("Meeting"));
        // The strip_alarms() method is called, which recreates the event without alarms
    }

    #[test]
    fn test_combined_modifiers() {
        let configs = vec![
            ModifierConfig::Replace {
                pattern: "🔔".to_string(),
                replacement: "".to_string(),
                field: "summary".to_string(),
            },
            ModifierConfig::StripReminders,
        ];

        let modifiers = CompiledModifier::compile_many(&configs).unwrap();
        let mut event = create_event("🔔 Important Meeting");

        apply_modifiers(&mut event, &modifiers);

        assert_eq!(event.summary(), Some(" Important Meeting"));
    }

    #[test]
    fn test_multi_field_modifiers() {
        let configs = vec![
            ModifierConfig::Replace {
                pattern: "Meeting".to_string(),
                replacement: "Event".to_string(),
                field: "summary".to_string(),
            },
            ModifierConfig::Replace {
                pattern: "Room".to_string(),
                replacement: "Space".to_string(),
                field: "location".to_string(),
            },
            ModifierConfig::Replace {
                pattern: "discuss".to_string(),
                replacement: "talk about".to_string(),
                field: "description".to_string(),
            },
        ];

        let modifiers = CompiledModifier::compile_many(&configs).unwrap();
        let mut event = create_event_full(
            "Meeting with team",
            "Let's discuss the project",
            "Conference Room A",
        );

        apply_modifiers(&mut event, &modifiers);

        assert_eq!(event.summary(), Some("Event with team"));
        assert_eq!(event.description(), Some("Let's talk about the project"));
        assert_eq!(event.location(), Some("Conference Space A"));
    }
}
