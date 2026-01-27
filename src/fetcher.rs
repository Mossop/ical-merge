use std::time::Duration;

use reqwest::Client;

use crate::error::Result;

/// Normalize webcal:// and webcals:// URLs to http:// and https://
fn normalize_calendar_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("webcal://") {
        format!("http://{}", rest)
    } else if let Some(rest) = url.strip_prefix("webcals://") {
        format!("https://{}", rest)
    } else {
        url.to_string()
    }
}

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
        let normalized_url = normalize_calendar_url(url);
        let response = self.client.get(&normalized_url).send().await?;
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

    #[test]
    fn test_normalize_webcal_url() {
        assert_eq!(
            normalize_calendar_url("webcal://example.com/cal.ics"),
            "http://example.com/cal.ics"
        );
        assert_eq!(
            normalize_calendar_url("webcals://example.com/cal.ics"),
            "https://example.com/cal.ics"
        );
        assert_eq!(
            normalize_calendar_url("http://example.com/cal.ics"),
            "http://example.com/cal.ics"
        );
        assert_eq!(
            normalize_calendar_url("https://example.com/cal.ics"),
            "https://example.com/cal.ics"
        );
    }

    #[tokio::test]
    async fn test_fetch_webcal_url() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/test.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ICAL))
            .mount(&mock_server)
            .await;

        let fetcher = Fetcher::new().unwrap();

        // Replace http:// with webcal:// in the URL
        let http_url = format!("{}/test.ics", mock_server.uri());
        let webcal_url = http_url.replace("http://", "webcal://");

        let result = fetcher.fetch(&webcal_url).await;
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("Test Event"));
    }

    #[tokio::test]
    async fn test_fetch_webcals_url() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/test.ics"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_ICAL))
            .mount(&mock_server)
            .await;

        let _fetcher = Fetcher::new().unwrap();

        // For webcals, we need to test with the http URL from mock server
        // (mock server uses http, but we're testing the URL normalization logic)
        let http_url = format!("{}/test.ics", mock_server.uri());
        let webcals_url = http_url.replace("http://", "webcals://");

        // This will normalize webcals:// to https://, but the mock server is http
        // So this test verifies the normalization happens, even if the request might fail
        // For a proper test, we'd need a mock server that supports https
        let normalized = normalize_calendar_url(&webcals_url);
        assert!(normalized.starts_with("https://"));
    }
}
