#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub id: SessionId,
    pub connected: bool,
    pub server: String,
}

impl SessionState {
    pub fn new(id: impl Into<String>, server: impl Into<String>) -> Self {
        Self { id: SessionId(id.into()), connected: false, server: server.into() }
    }

    pub fn reconnect(&mut self) { self.connected = true; }
    pub fn disconnect(&mut self) { self.connected = false; }
}
