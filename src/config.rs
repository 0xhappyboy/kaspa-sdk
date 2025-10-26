/// configuration module
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaspaConfig {
    pub rpc_url: String,
    pub ws_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub timeout_seconds: u64,
    pub cache_enabled: bool,
    pub cache_ttl_seconds: u64,
    pub monitored_addresses: Vec<String>,
    pub auto_reconnect: bool,
    pub max_reconnect_attempts: usize,
}

impl Default for KaspaConfig {
    fn default() -> Self {
        Self {
            rpc_url: "http://localhost:16110".to_string(),
            ws_url: "ws://localhost:16110".to_string(),
            username: None,
            password: None,
            timeout_seconds: 30,
            cache_enabled: true,
            cache_ttl_seconds: 300,
            monitored_addresses: Vec::new(),
            auto_reconnect: true,
            max_reconnect_attempts: 10,
        }
    }
}

impl KaspaConfig {
    pub fn load_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: KaspaConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save_to_file(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn with_rpc_url(mut self, url: &str) -> Self {
        self.rpc_url = url.to_string();
        self.ws_url = url.replace("http", "ws");
        self
    }

    pub fn with_credentials(mut self, username: &str, password: &str) -> Self {
        self.username = Some(username.to_string());
        self.password = Some(password.to_string());
        self
    }
}
