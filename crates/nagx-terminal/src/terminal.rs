#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSize { pub cols: u16, pub rows: u16 }

impl Default for TerminalSize {
    fn default() -> Self { Self { cols: 120, rows: 32 } }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Input(Vec<u8>),
    Output(Vec<u8>),
    Resize(TerminalSize),
    Signal(Signal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal { Interrupt, Suspend, Quit, Terminate }
