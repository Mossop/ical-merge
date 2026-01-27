use figment::{
    Figment,
    providers::{Env, Format, Json},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    pub calendars: HashMap<String, CalendarConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            port: default_port(),
        }
    }
}

fn default_bind_address() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalendarConfig {
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceConfig {
    pub url: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// Match mode for allow/deny steps
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    #[default]
    Any,
    All,
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
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Figment::new()
            .merge(Json::file(path.as_ref()))
            .merge(Env::prefixed("ICAL_MERGE_"))
            .extract()
            .map_err(|e| Error::Config(e.to_string()))
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
                if source.url.is_empty() {
                    return Err(Error::Config(format!(
                        "Calendar '{}' source {} has empty URL",
                        id, idx
                    )));
                }

                // Validate source steps
                Self::validate_steps(&source.steps, &format!("Calendar '{}' source {}", id, idx))?;
            }

            // Validate calendar-level steps
            Self::validate_steps(&calendar.steps, &format!("Calendar '{}'", id))?;
        }

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
            "server": {
                "bind_address": "0.0.0.0",
                "port": 9090
            },
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
        assert_eq!(config.server.bind_address, "0.0.0.0");
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.calendars.len(), 1);
        assert!(config.calendars.contains_key("test"));

        fs::remove_file(config_path).unwrap();
    }

    #[test]
    fn test_config_defaults() {
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
        let config_path = temp_dir.join("test_config_defaults.json");
        fs::write(&config_path, config_json).unwrap();

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.server.bind_address, "127.0.0.1");
        assert_eq!(config.server.port, 8080);

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
        if let Step::Allow { fields, mode, .. } = &source.steps[0] {
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
            server: ServerConfig::default(),
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
        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };
        assert!(config.validate().is_err());

        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
                    url: "https://example.com/test.ics".to_string(),
                    steps: vec![],
                }],
                steps: vec![],
            },
        );
        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };
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
        if let Step::Replace {
            pattern,
            replacement,
            field,
        } = &source.steps[0]
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
                sources: vec![SourceConfig {
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
        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };
        assert!(config.validate().is_ok());

        // Test invalid regex
        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
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
        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };
        assert!(config.validate().is_err());

        // Test empty patterns
        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
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
        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };
        assert!(config.validate().is_err());

        // Test invalid strip field
        let mut calendars = HashMap::new();
        calendars.insert(
            "test".to_string(),
            CalendarConfig {
                sources: vec![SourceConfig {
                    url: "https://example.com/test.ics".to_string(),
                    steps: vec![Step::Strip {
                        field: "invalid".to_string(),
                    }],
                }],
                steps: vec![],
            },
        );
        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };
        assert!(config.validate().is_err());
    }
}
