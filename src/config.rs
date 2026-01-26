use figment::{
    providers::{Env, Format, Json},
    Figment,
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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceConfig {
    pub url: String,
    #[serde(default)]
    pub filters: FilterConfig,
    #[serde(default)]
    pub modifiers: Vec<ModifierConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub allow: Vec<FilterRule>,
    #[serde(default)]
    pub deny: Vec<FilterRule>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilterRule {
    pub pattern: String,
    #[serde(default = "default_filter_fields")]
    pub fields: Vec<String>,
}

fn default_filter_fields() -> Vec<String> {
    vec!["summary".to_string(), "description".to_string()]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModifierConfig {
    pub pattern: String,
    pub replacement: String,
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
    fn test_filter_fields_default() {
        let config_json = r#"{
            "calendars": {
                "test": {
                    "sources": [
                        {
                            "url": "https://example.com/test.ics",
                            "filters": {
                                "allow": [
                                    { "pattern": "meeting" }
                                ]
                            }
                        }
                    ]
                }
            }
        }"#;

        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_filter_defaults.json");
        fs::write(&config_path, config_json).unwrap();

        let config = Config::load(&config_path).unwrap();
        let source = &config.calendars["test"].sources[0];
        let rule = &source.filters.allow[0];
        assert_eq!(rule.fields, vec!["summary", "description"]);

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
        calendars.insert("test".to_string(), CalendarConfig { sources: vec![] });
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
                    filters: FilterConfig::default(),
                    modifiers: vec![],
                }],
            },
        );
        let config = Config {
            server: ServerConfig::default(),
            calendars,
        };
        assert!(config.validate().is_ok());
    }
}
