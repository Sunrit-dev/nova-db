use nova_storage::SyncMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Strongly-typed server configuration with sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub max_connections: usize,
    pub max_frame_size: usize,
    pub sync_mode: String,
    pub auth_token: Option<String>,
    pub web_port: Option<u16>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 7400,
            web_port: None,
            data_dir: PathBuf::from("./data"),
            max_connections: 1000,
            max_frame_size: 16 * 1024 * 1024, // 16 MB
            sync_mode: "always".to_string(),
            auth_token: None,
        }
    }
}

impl ServerConfig {
    pub fn get_sync_mode(&self) -> SyncMode {
        match self.sync_mode.to_lowercase().as_str() {
            "none" | "buffered" => SyncMode::None,
            _ => SyncMode::Always,
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn web_addr(&self) -> Option<String> {
        self.web_port.map(|p| format!("{}:{}", self.host, p))
    }
}
