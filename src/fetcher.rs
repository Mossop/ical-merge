use std::time::Duration;

use reqwest::Client;

use crate::error::Result;

/// HTTP fetcher for iCal calendars
pub struct Fetcher {
    client: Client,
}

impl Fetcher {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(format!(
                "ical-merge/{} (+https://github.com/user/ical-merge)",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;

        Ok(Self { client })
    }

    pub fn with_timeout(timeout: Duration) -> Result<Self> {
        let client = Client::builder()
            .timeout(timeout)
            .user_agent(format!(
                "ical-merge/{} (+https://github.com/user/ical-merge)",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;

        Ok(Self { client })
    }

    pub async fn fetch(&self, url: &str) -> Result<String> {
        let response = self.client.get(url).send().await?;
        let text = response.error_for_status()?.text().await?;
        Ok(text)
    }
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new().expect("Failed to create default fetcher")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SAMPLE_ICAL: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test//EN
BEGIN:VEVENT
UID:test@example.com
DTSTAMP:20231201T120000Z
DTSTART:20231201T140000Z
DTEND:20231201T150000Z
SUMMARY:Test Event
END:VEVENT
END:VCALENDAR"#;

    #[tokio::test]
    async fn test_fetch_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/test.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ICAL))
            .mount(&mock_server)
            .await;

        let fetcher = Fetcher::new().unwrap();
        let url = format!("{}/test.ics", mock_server.uri());
        let result = fetcher.fetch(&url).await;

        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("Test Event"));
    }

    #[tokio::test]
    async fn test_fetch_404_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/notfound.ics"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let fetcher = Fetcher::new().unwrap();
        let url = format!("{}/notfound.ics", mock_server.uri());
        let result = fetcher.fetch(&url).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_timeout() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/slow.ics"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(SAMPLE_ICAL)
                    .set_delay(Duration::from_secs(3)),
            )
            .mount(&mock_server)
            .await;

        let fetcher = Fetcher::with_timeout(Duration::from_millis(500)).unwrap();
        let url = format!("{}/slow.ics", mock_server.uri());
        let result = fetcher.fetch(&url).await;

        assert!(result.is_err());
    }
}
