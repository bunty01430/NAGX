pub const APP_NAME: &str = "NAGX";
pub const PROTOCOL_NAME: &str = "NXP";
pub const PROTOCOL_MAJOR: u32 = 1;
pub const PROTOCOL_MINOR: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState { Normal, Minimized, Maximized, Fullscreen, Hidden, Closed }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission { Read, Write, Execute, Admin }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workspace { Overview, Development, Monitoring, Operations }
