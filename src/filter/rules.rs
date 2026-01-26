use regex::Regex;

use crate::config::FilterConfig;
use crate::error::Result;
use crate::ical::Event;

/// A compiled filter rule with regex
#[derive(Debug)]
pub struct CompiledFilterRule {
    regex: Regex,
    fields: Vec<String>,
}

impl CompiledFilterRule {
    pub fn new(pattern: &str, fields: Vec<String>) -> Result<Self> {
        let regex = Regex::new(pattern)?;
        Ok(Self { regex, fields })
    }

    /// Check if this rule matches the given event
    pub fn matches(&self, event: &Event) -> bool {
        for field in &self.fields {
            let text = match field.as_str() {
                "summary" => event.summary(),
                "description" => event.description(),
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

/// A compiled filter with allow and deny rules
#[derive(Debug)]
pub struct CompiledFilter {
    allow_rules: Vec<CompiledFilterRule>,
    deny_rules: Vec<CompiledFilterRule>,
}

impl CompiledFilter {
    pub fn compile(config: &FilterConfig) -> Result<Self> {
        let allow_rules = config
            .allow
            .iter()
            .map(|rule| CompiledFilterRule::new(&rule.pattern, rule.fields.clone()))
            .collect::<Result<Vec<_>>>()?;

        let deny_rules = config
            .deny
            .iter()
            .map(|rule| CompiledFilterRule::new(&rule.pattern, rule.fields.clone()))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            allow_rules,
            deny_rules,
        })
    }

    /// Determine if an event should be included based on filter rules
    pub fn should_include(&self, event: &Event) -> bool {
        let has_allow = !self.allow_rules.is_empty();
        let has_deny = !self.deny_rules.is_empty();

        let matches_deny = self.deny_rules.iter().any(|r| r.matches(event));
        let matches_allow = self.allow_rules.iter().any(|r| r.matches(event));

        match (has_allow, has_deny) {
            (false, false) => true,                         // No rules = allow all
            (true, false) => matches_allow,                 // Only allow = must match
            (false, true) => !matches_deny,                 // Only deny = must not match
            (true, true) => !matches_deny && matches_allow, // Both = must match allow AND not match deny
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FilterConfig, FilterRule};
    use icalendar::Component;

    fn create_event(summary: &str, description: Option<&str>) -> Event {
        let mut event = icalendar::Event::new();
        event.summary(summary);
        if let Some(desc) = description {
            event.description(desc);
        }
        Event::new(event)
    }

    #[test]
    fn test_no_rules_allows_all() {
        let config = FilterConfig::default();
        let filter = CompiledFilter::compile(&config).unwrap();

        let event = create_event("Meeting", None);
        assert!(filter.should_include(&event));
    }

    #[test]
    fn test_only_allow_rules() {
        let config = FilterConfig {
            allow: vec![FilterRule {
                pattern: "(?i)meeting".to_string(),
                fields: vec!["summary".to_string(), "description".to_string()],
            }],
            deny: vec![],
        };
        let filter = CompiledFilter::compile(&config).unwrap();

        let event1 = create_event("Meeting with team", None);
        assert!(filter.should_include(&event1));

        let event2 = create_event("Lunch", None);
        assert!(!filter.should_include(&event2));

        let event3 = create_event("Lunch", Some("discuss meeting"));
        assert!(filter.should_include(&event3));
    }

    #[test]
    fn test_only_deny_rules() {
        let config = FilterConfig {
            allow: vec![],
            deny: vec![FilterRule {
                pattern: "(?i)optional".to_string(),
                fields: vec!["summary".to_string()],
            }],
        };
        let filter = CompiledFilter::compile(&config).unwrap();

        let event1 = create_event("Meeting", None);
        assert!(filter.should_include(&event1));

        let event2 = create_event("Optional meeting", None);
        assert!(!filter.should_include(&event2));

        // Deny only checks summary field
        let event3 = create_event("Meeting", Some("optional attendance"));
        assert!(filter.should_include(&event3));
    }

    #[test]
    fn test_both_allow_and_deny_rules() {
        let config = FilterConfig {
            allow: vec![FilterRule {
                pattern: "(?i)meeting".to_string(),
                fields: vec!["summary".to_string(), "description".to_string()],
            }],
            deny: vec![FilterRule {
                pattern: "(?i)optional".to_string(),
                fields: vec!["summary".to_string(), "description".to_string()],
            }],
        };
        let filter = CompiledFilter::compile(&config).unwrap();

        // Matches allow, doesn't match deny
        let event1 = create_event("Meeting with team", None);
        assert!(filter.should_include(&event1));

        // Doesn't match allow
        let event2 = create_event("Lunch", None);
        assert!(!filter.should_include(&event2));

        // Matches both allow and deny
        let event3 = create_event("Optional meeting", None);
        assert!(!filter.should_include(&event3));

        // Matches allow but deny in description
        let event4 = create_event("Meeting", Some("optional attendance"));
        assert!(!filter.should_include(&event4));
    }

    #[test]
    fn test_multi_field_matching() {
        let config = FilterConfig {
            allow: vec![FilterRule {
                pattern: "(?i)important".to_string(),
                fields: vec!["summary".to_string(), "description".to_string()],
            }],
            deny: vec![],
        };
        let filter = CompiledFilter::compile(&config).unwrap();

        let event1 = create_event("Important meeting", None);
        assert!(filter.should_include(&event1));

        let event2 = create_event("Meeting", Some("This is important"));
        assert!(filter.should_include(&event2));

        let event3 = create_event("Meeting", None);
        assert!(!filter.should_include(&event3));
    }

    #[test]
    fn test_empty_filter() {
        let config = FilterConfig {
            allow: vec![],
            deny: vec![],
        };
        let filter = CompiledFilter::compile(&config).unwrap();

        let event = create_event("Any event", None);
        assert!(filter.should_include(&event));
    }
}
