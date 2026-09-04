#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl ServerConfig {
    pub fn new(host: impl Into<String>, user: impl Into<String>) -> Self {
        Self { host: host.into(), port: 22, user: user.into() }
    }
}
