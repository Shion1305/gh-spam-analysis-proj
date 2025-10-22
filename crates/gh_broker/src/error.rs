use std::fmt;

use http::StatusCode;

#[derive(Debug)]
pub struct HttpStatusError {
    pub status: StatusCode,
}

impl HttpStatusError {
    pub fn new(status: StatusCode) -> Self {
        Self { status }
    }
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unexpected status {}", self.status)
    }
}

impl std::error::Error for HttpStatusError {}
