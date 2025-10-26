use std::fmt;

#[derive(Debug)]
pub enum KaspaError {
    HttpError(reqwest::Error),
    JsonError(serde_json::Error),
    RpcError { code: i32, message: String },
    InvalidResponse(String),
    ConnectionError(String),
    AuthenticationError,
    InvalidAddress(String),
    Custom(String),
}

impl fmt::Display for KaspaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KaspaError::HttpError(e) => write!(f, "HTTP request failed: {}", e),
            KaspaError::JsonError(e) => write!(f, "JSON serialization failed: {}", e),
            KaspaError::RpcError { code, message } => {
                write!(f, "RPC error: code={}, message={}", code, message)
            }
            KaspaError::InvalidResponse(msg) => write!(f, "Invalid response format: {}", msg),
            KaspaError::ConnectionError(msg) => write!(f, "Connection failed: {}", msg),
            KaspaError::AuthenticationError => write!(f, "Authentication failed"),
            KaspaError::InvalidAddress(msg) => write!(f, "Invalid address: {}", msg),
            KaspaError::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for KaspaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KaspaError::HttpError(e) => Some(e),
            KaspaError::JsonError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for KaspaError {
    fn from(err: reqwest::Error) -> Self {
        KaspaError::HttpError(err)
    }
}

impl From<serde_json::Error> for KaspaError {
    fn from(err: serde_json::Error) -> Self {
        KaspaError::JsonError(err)
    }
}

pub type Result<T> = std::result::Result<T, KaspaError>;
