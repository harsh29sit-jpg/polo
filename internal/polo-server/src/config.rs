use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub db_path: String,
    pub listen: SocketAddr,
    pub token: Option<String>,
    pub log_level: String,
    pub cors_origin: Option<String>,
    pub max_body_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: "polo.db".into(),
            listen: "0.0.0.0:5432".parse().unwrap(),
            token: None,
            log_level: "info".into(),
            cors_origin: None,
            max_body_bytes: 4 * 1024 * 1024, // 4 MiB
        }
    }
}
