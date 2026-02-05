use figment::{
    Figment,
    providers::{Format, Json, Toml},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub calendars: HashMap<String, CalendarConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalendarConfig {
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SourceConfig {
    Url {
        url: String,
        #[serde(default)]
        steps: Vec<Step>,
    },
    Calendar {
        calendar: String,
        #[serde(default)]
        steps: Vec<Step>,
    },
}

impl SourceConfig {
    /// Get the steps for this source
    pub fn steps(&self) -> &[Step] {
        match self {
            SourceConfig::Url { steps, .. } => steps,
            SourceConfig::Calendar { steps, .. } => steps,
        }
    }

    /// Get an identifier for this source (URL or calendar reference)
    pub fn identifier(&self) -> String {
        match self {
            SourceConfig::Url { url, .. } => url.clone(),
            SourceConfig::Calendar { calendar, .. } => format!("calendar:{}", calendar),
        }
    }
}

/// Match mode for allow/deny steps
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    #[default]
    Any,
    All,
}

/// Case transformation mode
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseTransform {
    Lower,
    Upper,
    Sentence,
    Title,
}

fn default_step_fields() -> Vec<String> {
    vec!["summary".to_string(), "description".to_string()]
}

fn default_step_field() -> String {
    "summary".to_string()
}

fn default_replacement() -> String {
    String::new()
}

/// Processing step configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Step {
    Allow {
        patterns: Vec<String>,
        #[serde(default)]
        mode: MatchMode,
        #[serde(default = "default_step_fields")]
        fields: Vec<String>,
    },
    Deny {
        patterns: Vec<String>,
        #[serde(default)]
        mode: MatchMode,
        #[serde(default = "default_step_fields")]
        fields: Vec<String>,
    },
    Replace {
        pattern: String,
        #[serde(default = "default_replacement")]
        replacement: String,
        #[serde(default = "default_step_field")]
        field: String,
    },
    Strip {
        field: String,
    },
    Case {
        transform: CaseTransform,
        #[serde(default = "default_step_field")]
        field: String,
    },
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("json");

        let figment = Figment::new();
        let figment = match extension {
            "toml" => figment.merge(Toml::file(path)),
            _ => figment.merge(Json::file(path)),
        };

        figment.extract().map_err(|e| Error::Config(e.to_string()))
    }

    pub fn validate(&self) -> Result<()> {
        if self.calendars.is_empty() {
            return Err(Error::Config("No calendars configured".to_string()));
        }

        for (id, calendar) in &self.calendars {
            if calendar.sources.is_empty() {
                return Err(Error::Config(format!("Calendar '{}' has no sources", id)));
            }

            for (idx, source) in calendar.sources.iter().enumerate() {
                match source {
                    SourceConfig::Url { url, steps } => {
                        if url.is_empty() {
                            return Err(Error::Config(format!(
                                "Calendar '{}' source {} has empty URL",
                                id, idx
                            )));
                        }
                        // Validate source steps
                        Self::validate_steps(steps, &format!("Calendar '{}' source {}", id, idx))?;
                    }
                    SourceConfig::Calendar {
                        calendar: ref_id,
                        steps,
                    } => {
                        if ref_id.is_empty() {
                            return Err(Error::Config(format!(
                                "Calendar '{}' source {} has empty calendar reference",
                                id, idx
                            )));
                        }
                        // Check that referenced calendar exists
                        if !self.calendars.contains_key(ref_id) {
                            return Err(Error::Config(format!(
                                "Calendar '{}' source {} references unknown calendar '{}'",
                                id, idx, ref_id
                            )));
                        }
                        // Validate source steps
                        Self::validate_steps(steps, &format!("Calendar '{}' source {}", id, idx))?;
                    }
                }
            }

            // Validate calendar-level steps
            Self::validate_steps(&calendar.steps, &format!("Calendar '{}'", id))?;
        }

        // Detect cycles in calendar references
        for id in self.calendars.keys() {
            self.detect_cycle(
                id,
                &mut std::collections::HashSet::new(),
                &mut std::collections::HashSet::new(),
            )?;
        }

        Ok(())
    }

    /// Detect cycles in calendar references using DFS
    fn detect_cycle(
        &self,
        calendar_id: &str,
        visited: &mut std::collections::HashSet<String>,
        stack: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        if stack.contains(calendar_id) {
            return Err(Error::Config(format!(
                "Circular calendar reference detected involving '{}'",
                calendar_id
            )));
        }

        if visited.contains(calendar_id) {
            return Ok(());
        }

        visited.insert(calendar_id.to_string());
        stack.insert(calendar_id.to_string());

        if let Some(calendar) = self.calendars.get(calendar_id) {
            for source in &calendar.sources {
                if let SourceConfig::Calendar {
                    calendar: ref_id, ..
                } = source
                {
                    self.detect_cycle(ref_id, visited, stack)?;
                }
            }
        }

        stack.remove(calendar_id);
        Ok(())
    }

    fn validate_steps(steps: &[Step], context: &str) -> Result<()> {
        use regex::Regex;

        for (idx, step) in steps.iter().enumerate() {
            match step {
                Step::Allow { patterns, .. } | Step::Deny { patterns, .. } => {
                    if patterns.is_empty() {
                        return Err(Error::Config(format!(
                            "{} step {} has no patterns",
                            context, idx
                        )));
                    }
                    for pattern in patterns {
                        Regex::new(pattern).map_err(|e| {
                            Error::Config(format!(
                                "{} step {} has invalid pattern '{}': {}",
                                context, idx, pattern, e
                            ))
                        })?;
                    }
                }
                Step::Replace { pattern, .. } => {
                    Regex::new(pattern).map_err(|e| {
                        Error::Config(format!(
                            "{} step {} has invalid pattern '{}': {}",
                            context, idx, pattern, e
                        ))
                    })?;
                }
                Step::Strip { field } => {
                    if field != "reminder" {
                        return Err(Error::Config(format!(
                            "{} step {} has unsupported strip field '{}' (only 'reminder' is supported)",
                            context, idx, field
                        )));
                    }
                }
                Step::Case { .. } => {
                    // No validation needed for case transformation
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_config_parsing() {
        let config_json = r#"{
            "calendars": {
                "test": {
                    "sources": [
                        {
                            "url": "https://example.com/test.ics"
                        }
                    ]
                }
            }
        }"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_config.json");
        fs::write(&config_path, config_json).unwrap();

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.calendars.len(), 1);
        assert!(config.calendars.contains_key("test"));

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_config_minimal() {
        let config_json = r#"{
            "calendars": {
                "test": {
                    "sources": [
                        {
                            "url": "https://example.com/test.ics"
                        }
                    ]
                }
            }
        }"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_config_minimal.json");
        fs::write(&config_path, config_json).unwrap();

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.calendars.len(), 1);
        assert!(config.calendars.contains_key("test"));

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_step_fields_default() {
        let config_json = r#"{
            "calendars": {
                "test": {
                    "sources": [
                        {
                            "url": "https://example.com/test.ics",
                            "steps": [
                                {
                                    "type": "allow",
                                    "patterns": ["meeting"]
                                }
                            ]
                        }
                    ]
                }
            }
        }"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_step_defaults.json");
        fs::write(&config_path, config_json).unwrap();

        let config = Config::load(&config_path).unwrap();
        let source = &config.calendars["test"].sources[0];
        let steps = source.steps();
        if let Step::Allow { fields, mode, .. } = &steps[0] {
            assert_eq!(
                fields,
                &vec!["summary".to_string(), "description".to_string()]
            );
            assert!(matches!(mode, MatchMode::Any));
        } else {
            panic!("Expected Allow step");
        }

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_config_validation() {
        let config = Config {
            calendars: HashMap::new(),
        };
        assert!(config.validate().is_err());

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_err());

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: "https://example.com/test.ics".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_replace_step_default_replacement() {
        let config_json = r#"{
            "calendars": {
                "test": {
                    "sources": [
                        {
                            "url": "https://example.com/test.ics",
                            "steps": [
                                {
                                    "type": "replace",
                                    "pattern": "🔔"
                                }
                            ]
                        }
                    ]
                }
            }
        }"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_replace_default.json");
        fs::write(&config_path, config_json).unwrap();

        let config = Config::load(&config_path).unwrap();
        let source = &config.calendars["test"].sources[0];
        let steps = source.steps();
        if let Step::Replace {
            pattern,
            replacement,
            field,
        } = &steps[0]
        {
            assert_eq!(pattern, "🔔");
            assert_eq!(replacement, ""); // Default empty string
            assert_eq!(field, "summary"); // Default field
        } else {
            panic!("Expected Replace step");
        }

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_step_validation() {
        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: "https://example.com/test.ics".to_string(),
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
        assert!(config.validate().is_ok());

        // Test invalid regex
        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: "https://example.com/test.ics".to_string(),
                    steps: vec![Step::Allow {
                        patterns: vec!["[invalid".to_string()],
                        mode: MatchMode::Any,
                        fields: vec!["summary".to_string()],
                    }],
                }],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_err());

        // Test empty patterns
        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: "https://example.com/test.ics".to_string(),
                    steps: vec![Step::Allow {
                        patterns: vec![],
                        mode: MatchMode::Any,
                        fields: vec!["summary".to_string()],
                    }],
                }],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_err());

        // Test invalid strip field
        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: "https://example.com/test.ics".to_string(),
                    steps: vec![Step::Strip {
                        field: "invalid".to_string(),
                    }],
                }],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_calendar_reference_validation() {
        // Valid calendar reference
        let mut calendars = HashMap::new();
        calendars.insert(
            "base".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: "https://example.com/base.ics".to_string(),
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
        assert!(config.validate().is_ok());

        // Unknown calendar reference
        let mut calendars = HashMap::new();
        calendars.insert(
            "derived".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "nonexistent".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cycle_detection_direct() {
        // Direct self-reference A→A
        let mut calendars = HashMap::new();
        calendars.insert(
            "a".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "a".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cycle_detection_indirect() {
        // Indirect cycle A→B→A
        let mut calendars = HashMap::new();
        calendars.insert(
            "a".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "b".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "b".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "a".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_diamond_dependency() {
        // Diamond dependency A→B, A→C, B→D, C→D (valid)
        let mut calendars = HashMap::new();
        calendars.insert(
            "d".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Url {
                    url: "https://example.com/d.ics".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "b".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "d".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "c".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig::Calendar {
                    calendar: "d".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        calendars.insert(
            "a".to_string(),
            CalendarConfig {
                sources: vec![
                    SourceConfig::Calendar {
                        calendar: "b".to_string(),
                        steps: vec![],
                    },
                    SourceConfig::Calendar {
                        calendar: "c".to_string(),
                        steps: vec![],
                    },
                ],
                steps: vec![],
            },
        );
        let config = Config { calendars };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_parsing_toml() {
        let config_toml = r#"
[calendars.test]

[[calendars.test.sources]]
url = "https://example.com/test.ics"
steps = []

calendars.test.steps = []
"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_config.toml");
        fs::write(&config_path, config_toml).unwrap();

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.calendars.len(), 1);
        assert!(config.calendars.contains_key("test"));

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_config_toml_with_steps() {
        let config_toml = r#"
[calendars.test]

[[calendars.test.sources]]
url = "https://example.com/test.ics"

[[calendars.test.sources.steps]]
type = "allow"
patterns = ["(?i)meeting"]
mode = "any"
fields = ["summary"]

[[calendars.test.sources.steps]]
type = "replace"
pattern = "^"
replacement = "[TEST] "
field = "summary"

[[calendars.test.sources.steps]]
type = "case"
transform = "title"

calendars.test.steps = []
"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_config_steps.toml");
        fs::write(&config_path, config_toml).unwrap();

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.calendars.len(), 1);

        let calendar = config.calendars.get("test").unwrap();
        assert_eq!(calendar.sources.len(), 1);

        let steps = calendar.sources[0].steps();
        assert_eq!(steps.len(), 3);

        // Verify allow step
        match &steps[0] {
            Step::Allow {
                patterns,
                mode,
                fields,
            } => {
                assert_eq!(patterns.len(), 1);
                assert_eq!(patterns[0], "(?i)meeting");
                assert!(matches!(mode, MatchMode::Any));
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0], "summary");
            }
            _ => panic!("Expected Allow step"),
        }

        // Verify replace step
        match &steps[1] {
            Step::Replace {
                pattern,
                replacement,
                field,
            } => {
                assert_eq!(pattern, "^");
                assert_eq!(replacement, "[TEST] ");
                assert_eq!(field, "summary");
            }
            _ => panic!("Expected Replace step"),
        }

        // Verify case step
        match &steps[2] {
            Step::Case { transform, field } => {
                assert!(matches!(transform, CaseTransform::Title));
                assert_eq!(field, "summary");
            }
            _ => panic!("Expected Case step"),
        }

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_config_toml_calendar_reference() {
        let config_toml = r#"
[calendars.base]

[[calendars.base.sources]]
url = "https://example.com/base.ics"
steps = []

calendars.base.steps = []

[calendars.derived]

[[calendars.derived.sources]]
calendar = "base"

[[calendars.derived.sources.steps]]
type = "replace"
pattern = "^"
replacement = "[DERIVED] "
field = "summary"

calendars.derived.steps = []
"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_config_ref.toml");
        fs::write(&config_path, config_toml).unwrap();

        let config = Config::load(&config_path).unwrap();
        config.validate().unwrap();

        assert_eq!(config.calendars.len(), 2);
        assert!(config.calendars.contains_key("base"));
        assert!(config.calendars.contains_key("derived"));

        let derived = config.calendars.get("derived").unwrap();
        assert_eq!(derived.sources.len(), 1);

        match &derived.sources[0] {
            SourceConfig::Calendar { calendar, steps } => {
                assert_eq!(calendar, "base");
                assert_eq!(steps.len(), 1);
            }
            _ => panic!("Expected Calendar source"),
        }

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_example_configs_parse_correctly() {
        // Test config.example.json
        let json_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.json");
        let json_config =
            Config::load(&json_path).expect("config.example.json should parse correctly");
        json_config
            .validate()
            .expect("config.example.json should be valid");

        // Verify basic structure of example JSON config
        assert!(
            json_config.calendars.contains_key("combined-work"),
            "config.example.json should contain 'combined-work' calendar"
        );
        assert!(
            json_config.calendars.contains_key("personal"),
            "config.example.json should contain 'personal' calendar"
        );
        assert!(
            json_config.calendars.contains_key("everything"),
            "config.example.json should contain 'everything' calendar"
        );

        // Test config.example.toml
        let toml_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
        let toml_config =
            Config::load(&toml_path).expect("config.example.toml should parse correctly");
        toml_config
            .validate()
            .expect("config.example.toml should be valid");

        // Verify basic structure of example TOML config
        assert!(
            toml_config.calendars.contains_key("combined-work"),
            "config.example.toml should contain 'combined-work' calendar"
        );
        assert!(
            toml_config.calendars.contains_key("personal"),
            "config.example.toml should contain 'personal' calendar"
        );
        assert!(
            toml_config.calendars.contains_key("everything"),
            "config.example.toml should contain 'everything' calendar"
        );

        // Verify both configs have equivalent structure
        assert_eq!(
            json_config.calendars.len(),
            toml_config.calendars.len(),
            "JSON and TOML examples should have same number of calendars"
        );
    }
}
