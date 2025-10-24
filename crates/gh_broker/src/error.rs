use std::fmt;

use http::StatusCode;

#[derive(Debug)]
pub struct HttpStatusError {
    pub status: StatusCode,
    pub endpoint: String,
}

impl HttpStatusError {
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            endpoint: String::new(),
        }
    }

    pub fn with_endpoint(status: StatusCode, endpoint: impl Into<String>) -> Self {
        Self {
            status,
            endpoint: endpoint.into(),
        }
    }
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.endpoint.is_empty() {
            write!(f, "unexpected status {}", self.status)
        } else {
            write!(f, "unexpected status {} for {}", self.status, self.endpoint)
        }
    }
}

impl std::error::Error for HttpStatusError {}
