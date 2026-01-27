#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("HTTP fetch error: {0}")]
    Fetch(#[from] reqwest::Error),

    #[error("iCal parse error: {0}")]
    Parse(String),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("File watcher error: {0}")]
    Notify(#[from] notify::Error),

    #[error("Calendar not found: {0}")]
    CalendarNotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;
