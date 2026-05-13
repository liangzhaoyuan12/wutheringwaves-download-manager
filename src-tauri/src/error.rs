use serde::Serialize;
use std::fmt;

#[derive(Debug, Serialize)]
pub enum WwError {
    Network(String),
    Config(String),
    Generic(String),
}

impl fmt::Display for WwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WwError::Network(msg) => write!(f, "Network error: {}", msg),
            WwError::Config(msg) => write!(f, "Config error: {}", msg),
            WwError::Generic(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for WwError {}

impl From<reqwest::Error> for WwError {
    fn from(e: reqwest::Error) -> Self {
        WwError::Network(e.to_string())
    }
}

impl From<std::io::Error> for WwError {
    fn from(e: std::io::Error) -> Self {
        WwError::Generic(e.to_string())
    }
}

impl From<url::ParseError> for WwError {
    fn from(e: url::ParseError) -> Self {
        WwError::Network(format!("URL parse error: {}", e))
    }
}

impl From<serde_json::Error> for WwError {
    fn from(e: serde_json::Error) -> Self {
        WwError::Generic(format!("JSON error: {}", e))
    }
}
