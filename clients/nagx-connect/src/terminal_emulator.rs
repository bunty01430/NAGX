use vt100::Parser;

pub struct TerminalEmulator {
    parser: Parser,
}

impl TerminalEmulator {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self { parser: Parser::new(rows, cols, scrollback) }
    }

    pub fn process(&mut self, bytes: &[u8]) -> String {
        self.parser.process(bytes);
        self.render()
    }

    pub fn render(&self) -> String {
        self.parser.screen().contents()
    }

    pub fn render_formatted_basic(&self) -> String {
        self.parser.screen().contents_formatted_basic()
    }

    pub fn cursor_position(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        self.parser.screen_mut().set_size(
            std::num::NonZeroU16::new(rows).expect("rows clamped above 0"),
            std::num::NonZeroU16::new(cols).expect("cols clamped above 0"),
        );
    }

    pub fn set_scrollback(&mut self, rows: usize) {
        self.parser.screen_mut().set_scrollback(rows);
    }
}
