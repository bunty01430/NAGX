use vt100::{Color, MouseProtocolEncoding, MouseProtocolMode, Parser};

pub struct TerminalEmulator {
    parser: Parser,
}

impl TerminalEmulator {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self { parser: Parser::new(rows, cols, scrollback) }
    }

    pub fn process(&mut self, bytes: &[u8]) -> String {
        self.parser.process(bytes);
        self.render_markup()
    }

    pub fn render(&self) -> String { self.parser.screen().contents() }

    pub fn render_markup(&self) -> String {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let mut markup = String::with_capacity((rows as usize) * (cols as usize + 1));

        for row in 0..rows {
            let mut current_fg: Option<Color> = None;
            let mut current_bold = false;
            let mut current_italic = false;
            let mut run = String::new();

            for col in 0..cols {
                let cell = match screen.cell(row, col) {
                    Some(cell) => cell,
                    None => continue,
                };

                let fg = effective_color(cell.fgcolor(), cell.bold());
                let bold = cell.bold();
                let italic = cell.italic();

                if current_fg != Some(fg) || current_bold != bold || current_italic != italic {
                    if !run.is_empty() {
                        append_span(&mut markup, current_fg, current_bold, current_italic, &run);
                        run.clear();
                    }
                    current_fg = Some(fg);
                    current_bold = bold;
                    current_italic = italic;
                }

                if cell.is_wide_continuation() { continue; }
                let value = if cell.has_contents() { cell.contents() } else { " " };
                escape_markup(value, &mut run);
            }

            if !run.is_empty() {
                append_span(&mut markup, current_fg, current_bold, current_italic, &run);
            }
            if row + 1 < rows { markup.push('\n'); }
        }

        markup
    }

    pub fn cursor_position(&self) -> (u16, u16) { self.parser.screen().cursor_position() }
    pub fn cursor_visible(&self) -> bool { !self.parser.screen().hide_cursor() }
    pub fn size(&self) -> (u16, u16) { self.parser.screen().size() }

    pub fn set_size(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        self.parser.screen_mut().set_size(
            std::num::NonZeroU16::new(rows).expect("rows clamped above 0"),
            std::num::NonZeroU16::new(cols).expect("cols clamped above 0"),
        );
    }

    pub fn set_scrollback(&mut self, rows: usize) { self.parser.screen_mut().set_scrollback(rows); }

    pub fn mouse_reporting_enabled(&self) -> bool {
        self.parser.screen().mouse_protocol_mode() != MouseProtocolMode::None
    }

    pub fn bracketed_paste_enabled(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    pub fn mouse_report(
        &self,
        button: u8,
        kind: u8,
        x: u16,
        y: u16,
        shift: bool,
        alt: bool,
        control: bool,
    ) -> Option<Vec<u8>> {
        if !self.mouse_reporting_enabled() { return None; }

        let mut code = match button {
            1 => 0,
            2 => 1,
            3 => 2,
            _ => 3,
        };

        if kind == 3 { code |= 32; }
        if shift { code |= 4; }
        if alt { code |= 8; }
        if control { code |= 16; }

        if matches!(button, 4 | 5) {
            code = if button == 4 { 64 } else { 65 };
            if shift { code |= 4; }
            if alt { code |= 8; }
            if control { code |= 16; }
        }

        match self.parser.screen().mouse_protocol_encoding() {
            MouseProtocolEncoding::Sgr => {
                let suffix = if kind == 2 { 'm' } else { 'M' };
                Some(format!("\x1b[<{};{};{}{}", code, x.max(1), y.max(1), suffix).into_bytes())
            }
            MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
                if x > 223 || y > 223 { return None; }
                Some(vec![0x1b, b'[', b'M', 32 + code, 32 + x as u8, 32 + y as u8])
            }
        }
    }
}

fn effective_color(color: Color, bold: bool) -> Color {
    match color {
        Color::Idx(index) if bold && index < 8 => Color::Idx(index + 8),
        other => other,
    }
}

fn append_span(markup: &mut String, color: Option<Color>, bold: bool, italic: bool, text: &str) {
    let color_text = color.map(color_hex).unwrap_or_else(|| "#d7dee8".to_string());
    markup.push_str("<font color='");
    markup.push_str(&color_text);
    markup.push_str("'>");
    if bold { markup.push_str("**"); }
    if italic { markup.push('*'); }
    markup.push_str(text);
    if italic { markup.push('*'); }
    if bold { markup.push_str("**"); }
    markup.push_str("</font>");
}

fn color_hex(color: Color) -> String {
    let (r, g, b) = match color {
        Color::Default => (215, 222, 232),
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Idx(index) => xterm_palette(index),
    };
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn xterm_palette(index: u8) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0),
        (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
        (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0),
        (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let n = index - 16;
            (cube_value(n / 36), cube_value((n % 36) / 6), cube_value(n % 6))
        }
        _ => {
            let shade = 8 + (index - 232) * 10;
            (shade, shade, shade)
        }
    }
}

fn cube_value(value: u8) -> u8 { if value == 0 { 0 } else { 55 + value * 40 } }

fn escape_markup(input: &str, output: &mut String) {
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '*' => output.push_str("\\*"),
            '_' => output.push_str("\\_"),
            '`' => output.push_str("\\`"),
            '[' => output.push_str("\\["),
            ']' => output.push_str("\\]"),
            _ => output.push(ch),
        }
    }
}
