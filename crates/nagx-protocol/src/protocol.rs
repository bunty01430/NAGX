pub const PROTOCOL_VERSION: &str = "1.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageKind { Hello, Welcome, ResizePty, TerminalInput, TerminalOutput, Ping, Pong }
