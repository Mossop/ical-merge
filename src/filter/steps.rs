use regex::Regex;

use crate::config::{CaseTransform, MatchMode, Step};
use crate::error::Result;
use crate::ical::Event;

/// A compiled pattern with associated fields
#[derive(Debug)]
pub struct CompiledPattern {
    regex: Regex,
    fields: Vec<String>,
}

impl CompiledPattern {
    pub fn new(pattern: &str, fields: Vec<String>) -> Result<Self> {
        let regex = Regex::new(pattern)?;
        Ok(Self { regex, fields })
    }

    /// Check if this pattern matches any of the specified fields in the event
    pub fn matches(&self, event: &Event) -> bool {
        for field in &self.fields {
            let text = match field.as_str() {
                "summary" => event.summary(),
                "description" => event.description(),
                "location" => event.location(),
                _ => None,
            };

            if let Some(text) = text
                && self.regex.is_match(text)
            {
                return true;
            }
        }

        false
    }
}

/// Result of applying a step
#[derive(Debug, PartialEq)]
pub enum StepResult {
    Keep,
    Reject,
}

/// A compiled step with pre-compiled regexes
#[derive(Debug)]
pub enum CompiledStep {
    Allow {
        patterns: Vec<CompiledPattern>,
        mode: MatchMode,
    },
    Deny {
        patterns: Vec<CompiledPattern>,
        mode: MatchMode,
    },
    Replace {
        regex: Regex,
        replacement: String,
        field: String,
    },
    Strip {
        field: String,
    },
    Case {
        transform: CaseTransform,
        field: String,
    },
}

impl CompiledStep {
    /// Compile a single step
    pub fn compile(step: &Step) -> Result<Self> {
        match step {
            Step::Allow {
                patterns,
                mode,
                fields,
            } => {
                let compiled = patterns
                    .iter()
                    .map(|p| CompiledPattern::new(p, fields.clone()))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Self::Allow {
                    patterns: compiled,
                    mode: mode.clone(),
                })
            }
            Step::Deny {
                patterns,
                mode,
                fields,
            } => {
                let compiled = patterns
                    .iter()
                    .map(|p| CompiledPattern::new(p, fields.clone()))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Self::Deny {
                    patterns: compiled,
                    mode: mode.clone(),
                })
            }
            Step::Replace {
                pattern,
                replacement,
                field,
            } => {
                let regex = Regex::new(pattern)?;
                Ok(Self::Replace {
                    regex,
                    replacement: replacement.clone(),
                    field: field.clone(),
                })
            }
            Step::Strip { field } => Ok(Self::Strip {
                field: field.clone(),
            }),
            Step::Case { transform, field } => Ok(Self::Case {
                transform: transform.clone(),
                field: field.clone(),
            }),
        }
    }

    /// Compile multiple steps
    pub fn compile_many(steps: &[Step]) -> Result<Vec<Self>> {
        steps.iter().map(Self::compile).collect()
    }

    /// Apply this step to an event
    pub fn apply(&self, event: &mut Event) -> StepResult {
        match self {
            Self::Allow { patterns, mode } => {
                let matches: Vec<bool> = patterns.iter().map(|p| p.matches(event)).collect();

                let passes = match mode {
                    MatchMode::Any => matches.iter().any(|&m| m),
                    MatchMode::All => matches.iter().all(|&m| m),
                };

                if passes {
                    StepResult::Keep
                } else {
                    StepResult::Reject
                }
            }
            Self::Deny { patterns, mode } => {
                let matches: Vec<bool> = patterns.iter().map(|p| p.matches(event)).collect();

                let blocked = match mode {
                    MatchMode::Any => matches.iter().any(|&m| m),
                    MatchMode::All => matches.iter().all(|&m| m),
                };

                if blocked {
                    StepResult::Reject
                } else {
                    StepResult::Keep
                }
            }
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

                StepResult::Keep
            }
            Self::Strip { field } => {
                if field.as_str() == "reminder" {
                    event.strip_alarms()
                }

                StepResult::Keep
            }
            Self::Case { transform, field } => {
                let text = match field.as_str() {
                    "summary" => event.summary().map(|s| s.to_string()),
                    "description" => event.description().map(|s| s.to_string()),
                    "location" => event.location().map(|s| s.to_string()),
                    _ => None,
                };

                if let Some(text) = text {
                    let new_text = match transform {
                        CaseTransform::Lower => text.to_lowercase(),
                        CaseTransform::Upper => text.to_uppercase(),
                        CaseTransform::Sentence => {
                            let mut chars = text.chars();
                            match chars.next() {
                                None => String::new(),
                                Some(first) => {
                                    first.to_uppercase().collect::<String>()
                                        + &chars.as_str().to_lowercase()
                                }
                            }
                        }
                        CaseTransform::Title => text
                            .split_whitespace()
                            .map(|word| {
                                let mut chars = word.chars();
                                match chars.next() {
                                    None => String::new(),
                                    Some(first) => {
                                        first.to_uppercase().collect::<String>()
                                            + &chars.as_str().to_lowercase()
                                    }
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" "),
                    };
                    match field.as_str() {
                        "summary" => event.set_summary(&new_text),
                        "description" => event.set_description(&new_text),
                        "location" => event.set_location(&new_text),
                        _ => {}
                    }
                }

                StepResult::Keep
            }
        }
    }
}

/// Apply all steps to an event, stopping at the first rejection
pub fn apply_steps(event: &mut Event, steps: &[CompiledStep]) -> StepResult {
    for step in steps {
        if step.apply(event) == StepResult::Reject {
            return StepResult::Reject;
        }
    }
    StepResult::Keep
}

/// Process events through a step pipeline, filtering and transforming them
pub fn process_events(events: Vec<Event>, steps: &[CompiledStep]) -> Vec<Event> {
    events
        .into_iter()
        .filter_map(|mut event| {
            if apply_steps(&mut event, steps) == StepResult::Keep {
                Some(event)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MatchMode, Step};
    use icalendar::{Component, EventLike};

    fn create_event(summary: &str, description: Option<&str>) -> Event {
        let mut event = icalendar::Event::new();
        event.summary(summary);
        if let Some(desc) = description {
            event.description(desc);
        }
        Event::new(event)
    }

    fn create_event_with_location(
        summary: &str,
        description: Option<&str>,
        location: Option<&str>,
    ) -> Event {
        let mut event = icalendar::Event::new();
        event.summary(summary);
        if let Some(desc) = description {
            event.description(desc);
        }
        if let Some(loc) = location {
            event.location(loc);
        }
        Event::new(event)
    }

    #[test]
    fn test_allow_step_any_mode() {
        let step = Step::Allow {
            patterns: vec!["(?i)meeting".to_string(), "(?i)standup".to_string()],
            mode: MatchMode::Any,
            fields: vec!["summary".to_string()],
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event1 = create_event("Meeting with team", None);
        assert_eq!(compiled.apply(&mut event1), StepResult::Keep);

        let mut event2 = create_event("Daily standup", None);
        assert_eq!(compiled.apply(&mut event2), StepResult::Keep);

        let mut event3 = create_event("Lunch", None);
        assert_eq!(compiled.apply(&mut event3), StepResult::Reject);
    }

    #[test]
    fn test_allow_step_all_mode() {
        let step = Step::Allow {
            patterns: vec!["(?i)important".to_string(), "(?i)meeting".to_string()],
            mode: MatchMode::All,
            fields: vec!["summary".to_string()],
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event1 = create_event("Important meeting", None);
        assert_eq!(compiled.apply(&mut event1), StepResult::Keep);

        let mut event2 = create_event("Important discussion", None);
        assert_eq!(compiled.apply(&mut event2), StepResult::Reject);

        let mut event3 = create_event("Regular meeting", None);
        assert_eq!(compiled.apply(&mut event3), StepResult::Reject);
    }

    #[test]
    fn test_deny_step_any_mode() {
        let step = Step::Deny {
            patterns: vec!["(?i)optional".to_string(), "(?i)canceled".to_string()],
            mode: MatchMode::Any,
            fields: vec!["summary".to_string()],
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event1 = create_event("Optional meeting", None);
        assert_eq!(compiled.apply(&mut event1), StepResult::Reject);

        let mut event2 = create_event("Canceled event", None);
        assert_eq!(compiled.apply(&mut event2), StepResult::Reject);

        let mut event3 = create_event("Regular meeting", None);
        assert_eq!(compiled.apply(&mut event3), StepResult::Keep);
    }

    #[test]
    fn test_deny_step_all_mode() {
        let step = Step::Deny {
            patterns: vec!["(?i)optional".to_string(), "(?i)meeting".to_string()],
            mode: MatchMode::All,
            fields: vec!["summary".to_string()],
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event1 = create_event("Optional meeting", None);
        assert_eq!(compiled.apply(&mut event1), StepResult::Reject);

        let mut event2 = create_event("Optional lunch", None);
        assert_eq!(compiled.apply(&mut event2), StepResult::Keep);

        let mut event3 = create_event("Regular meeting", None);
        assert_eq!(compiled.apply(&mut event3), StepResult::Keep);
    }

    #[test]
    fn test_replace_step() {
        let step = Step::Replace {
            pattern: "^Meeting:".to_string(),
            replacement: "[WORK]".to_string(),
            field: "summary".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("Meeting: Team sync", None);
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.summary(), Some("[WORK] Team sync"));
    }

    #[test]
    fn test_replace_step_empty_replacement() {
        // Test that empty replacement removes the matched text
        let step = Step::Replace {
            pattern: "🔔 ".to_string(),
            replacement: "".to_string(),
            field: "summary".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("🔔 Important Meeting", None);
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.summary(), Some("Important Meeting"));
    }

    #[test]
    fn test_strip_step() {
        let step = Step::Strip {
            field: "reminder".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("Meeting", None);
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
    }

    #[test]
    fn test_step_ordering() {
        // Allow then replace
        let steps = vec![
            Step::Allow {
                patterns: vec!["(?i)meeting".to_string()],
                mode: MatchMode::Any,
                fields: vec!["summary".to_string()],
            },
            Step::Replace {
                pattern: "Meeting".to_string(),
                replacement: "[WORK]".to_string(),
                field: "summary".to_string(),
            },
        ];
        let compiled = CompiledStep::compile_many(&steps).unwrap();

        let mut event1 = create_event("Meeting with team", None);
        assert_eq!(apply_steps(&mut event1, &compiled), StepResult::Keep);
        assert_eq!(event1.summary(), Some("[WORK] with team"));

        let mut event2 = create_event("Lunch", None);
        assert_eq!(apply_steps(&mut event2, &compiled), StepResult::Reject);
    }

    #[test]
    fn test_replace_then_allow() {
        // Replace then allow - shows order matters
        let steps = vec![
            Step::Replace {
                pattern: "Meeting".to_string(),
                replacement: "Event".to_string(),
                field: "summary".to_string(),
            },
            Step::Allow {
                patterns: vec!["Event".to_string()],
                mode: MatchMode::Any,
                fields: vec!["summary".to_string()],
            },
        ];
        let compiled = CompiledStep::compile_many(&steps).unwrap();

        // "Meeting" gets replaced to "Event", then allow checks for "Event"
        let mut event = create_event("Meeting with team", None);
        assert_eq!(apply_steps(&mut event, &compiled), StepResult::Keep);
        assert_eq!(event.summary(), Some("Event with team"));
    }

    #[test]
    fn test_process_events() {
        let steps = vec![
            Step::Allow {
                patterns: vec!["(?i)meeting".to_string()],
                mode: MatchMode::Any,
                fields: vec!["summary".to_string()],
            },
            Step::Replace {
                pattern: "Meeting".to_string(),
                replacement: "[WORK]".to_string(),
                field: "summary".to_string(),
            },
        ];
        let compiled = CompiledStep::compile_many(&steps).unwrap();

        let events = vec![
            create_event("Meeting 1", None),
            create_event("Lunch", None),
            create_event("Meeting 2", None),
            create_event("Break", None),
        ];

        let processed = process_events(events, &compiled);

        assert_eq!(processed.len(), 2);
        assert_eq!(processed[0].summary(), Some("[WORK] 1"));
        assert_eq!(processed[1].summary(), Some("[WORK] 2"));
    }

    #[test]
    fn test_multi_field_matching() {
        let step = Step::Allow {
            patterns: vec!["(?i)important".to_string()],
            mode: MatchMode::Any,
            fields: vec!["summary".to_string(), "description".to_string()],
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event1 = create_event("Important meeting", None);
        assert_eq!(compiled.apply(&mut event1), StepResult::Keep);

        let mut event2 = create_event("Meeting", Some("This is important"));
        assert_eq!(compiled.apply(&mut event2), StepResult::Keep);

        let mut event3 = create_event("Meeting", None);
        assert_eq!(compiled.apply(&mut event3), StepResult::Reject);
    }

    #[test]
    fn test_location_field() {
        let step = Step::Allow {
            patterns: vec!["(?i)stadium".to_string()],
            mode: MatchMode::Any,
            fields: vec!["location".to_string()],
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event1 = create_event_with_location("Match", None, Some("Allianz Stadium"));
        assert_eq!(compiled.apply(&mut event1), StepResult::Keep);

        let mut event2 = create_event_with_location("Match", None, Some("Park"));
        assert_eq!(compiled.apply(&mut event2), StepResult::Reject);

        let mut event3 = create_event_with_location("Match", None, None);
        assert_eq!(compiled.apply(&mut event3), StepResult::Reject);
    }

    #[test]
    fn test_deny_then_allow() {
        // Deny optional, then allow meetings
        let steps = vec![
            Step::Deny {
                patterns: vec!["(?i)optional".to_string()],
                mode: MatchMode::Any,
                fields: vec!["summary".to_string()],
            },
            Step::Allow {
                patterns: vec!["(?i)meeting".to_string()],
                mode: MatchMode::Any,
                fields: vec!["summary".to_string()],
            },
        ];
        let compiled = CompiledStep::compile_many(&steps).unwrap();

        let mut event1 = create_event("Meeting", None);
        assert_eq!(apply_steps(&mut event1, &compiled), StepResult::Keep);

        let mut event2 = create_event("Optional meeting", None);
        assert_eq!(apply_steps(&mut event2, &compiled), StepResult::Reject);

        let mut event3 = create_event("Lunch", None);
        assert_eq!(apply_steps(&mut event3, &compiled), StepResult::Reject);
    }

    #[test]
    fn test_multiple_replacements() {
        let steps = vec![
            Step::Replace {
                pattern: "Meeting".to_string(),
                replacement: "Event".to_string(),
                field: "summary".to_string(),
            },
            Step::Replace {
                pattern: "Event".to_string(),
                replacement: "Activity".to_string(),
                field: "summary".to_string(),
            },
        ];
        let compiled = CompiledStep::compile_many(&steps).unwrap();

        let mut event = create_event("Meeting with team", None);
        assert_eq!(apply_steps(&mut event, &compiled), StepResult::Keep);
        assert_eq!(event.summary(), Some("Activity with team"));
    }

    #[test]
    fn test_replace_multiple_fields() {
        let steps = vec![
            Step::Replace {
                pattern: "Meeting".to_string(),
                replacement: "Event".to_string(),
                field: "summary".to_string(),
            },
            Step::Replace {
                pattern: "discuss".to_string(),
                replacement: "talk about".to_string(),
                field: "description".to_string(),
            },
            Step::Replace {
                pattern: "Room".to_string(),
                replacement: "Space".to_string(),
                field: "location".to_string(),
            },
        ];
        let compiled = CompiledStep::compile_many(&steps).unwrap();

        let mut event = icalendar::Event::new();
        event.summary("Meeting with team");
        event.description("Let's discuss the project");
        event.location("Conference Room A");
        let mut event = Event::new(event);

        assert_eq!(apply_steps(&mut event, &compiled), StepResult::Keep);
        assert_eq!(event.summary(), Some("Event with team"));
        assert_eq!(event.description(), Some("Let's talk about the project"));
        assert_eq!(event.location(), Some("Conference Space A"));
    }

    #[test]
    fn test_case_lower() {
        let step = Step::Case {
            transform: CaseTransform::Lower,
            field: "summary".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("Meeting With TEAM", None);
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.summary(), Some("meeting with team"));
    }

    #[test]
    fn test_case_upper() {
        let step = Step::Case {
            transform: CaseTransform::Upper,
            field: "summary".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("meeting with team", None);
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.summary(), Some("MEETING WITH TEAM"));
    }

    #[test]
    fn test_case_sentence() {
        let step = Step::Case {
            transform: CaseTransform::Sentence,
            field: "summary".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("MEETING WITH TEAM", None);
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.summary(), Some("Meeting with team"));

        let mut event2 = create_event("meeting with team", None);
        assert_eq!(compiled.apply(&mut event2), StepResult::Keep);
        assert_eq!(event2.summary(), Some("Meeting with team"));
    }

    #[test]
    fn test_case_title() {
        let step = Step::Case {
            transform: CaseTransform::Title,
            field: "summary".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("meeting with team", None);
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.summary(), Some("Meeting With Team"));

        let mut event2 = create_event("MEETING WITH TEAM", None);
        assert_eq!(compiled.apply(&mut event2), StepResult::Keep);
        assert_eq!(event2.summary(), Some("Meeting With Team"));

        let mut event3 = create_event("meeting WITH team", None);
        assert_eq!(compiled.apply(&mut event3), StepResult::Keep);
        assert_eq!(event3.summary(), Some("Meeting With Team"));
    }

    #[test]
    fn test_case_on_description() {
        let step = Step::Case {
            transform: CaseTransform::Upper,
            field: "description".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event("Meeting", Some("important discussion"));
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.summary(), Some("Meeting"));
        assert_eq!(event.description(), Some("IMPORTANT DISCUSSION"));
    }

    #[test]
    fn test_case_on_location() {
        let step = Step::Case {
            transform: CaseTransform::Lower,
            field: "location".to_string(),
        };
        let compiled = CompiledStep::compile(&step).unwrap();

        let mut event = create_event_with_location("Meeting", None, Some("Conference ROOM A"));
        assert_eq!(compiled.apply(&mut event), StepResult::Keep);
        assert_eq!(event.location(), Some("conference room a"));
    }
}
