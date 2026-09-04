use vt100::{Color, MouseProtocolEncoding, MouseProtocolMode, Parser};

pub struct TerminalEmulator { parser: Parser }

impl TerminalEmulator {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self { Self { parser: Parser::new(rows, cols, scrollback) } }
    pub fn process(&mut self, bytes: &[u8]) -> String { self.parser.process(bytes); self.render_markup() }
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
                let cell = match screen.cell(row, col) { Some(cell) => cell, None => continue };
                let fg = effective_color(cell.fgcolor(), cell.bold());
                let bold = cell.bold();
                let italic = cell.italic();
                if current_fg != Some(fg) || current_bold != bold || current_italic != italic {
                    if !run.is_empty() { append_span(&mut markup, current_fg, current_bold, current_italic, &run); run.clear(); }
                    current_fg = Some(fg); current_bold = bold; current_italic = italic;
                }
                if cell.is_wide_continuation() { continue; }
                let value = if cell.has_contents() { cell.contents() } else { " " };
                escape_markup(value, &mut run);
            }
            if !run.is_empty() { append_span(&mut markup, current_fg, current_bold, current_italic, &run); }
            if row + 1 < rows { markup.push('\n'); }
        }
        markup
    }
    pub fn cursor_position(&self) -> (u16, u16) { self.parser.screen().cursor_position() }
    pub fn cursor_visible(&self) -> bool { !self.parser.screen().hide_cursor() }
    pub fn size(&self) -> (u16, u16) { self.parser.screen().size() }
    pub fn set_size(&mut self, rows: u16, cols: u16) { self.parser.screen_mut().set_size(rows.max(1), cols.max(1)); }
    pub fn set_scrollback(&mut self, rows: usize) { self.parser.screen_mut().set_scrollback(rows); }
    pub fn mouse_reporting_enabled(&self) -> bool { self.parser.screen().mouse_protocol_mode() != MouseProtocolMode::None }
    pub fn bracketed_paste_enabled(&self) -> bool { self.parser.screen().bracketed_paste() }
    pub fn mouse_report(&self, button: u8, kind: u8, x: u16, y: u16, shift: bool, alt: bool, control: bool) -> Option<Vec<u8>> {
        if !self.mouse_reporting_enabled() { return None; }
        let mut code = match button { 1 => 0u16, 2 => 1u16, 3 => 2u16, 4 => 64u16, 5 => 65u16, _ => return None };
        if kind == 3 { code += 32; }
        if shift { code += 4; }
        if alt { code += 8; }
        if control { code += 16; }
        match self.parser.screen().mouse_protocol_encoding() {
            MouseProtocolEncoding::Sgr => {
                let suffix = if kind == 2 { 'm' } else { 'M' };
                Some(format!("\x1b[<{};{};{}{}", code, x, y, suffix).into_bytes())
            }
            MouseProtocolEncoding::Utf8 => {
                let mut data = Vec::new(); data.extend_from_slice(b"\x1b[M");
                push_utf8_code(&mut data, code); push_utf8_code(&mut data, x); push_utf8_code(&mut data, y); Some(data)
            }
            MouseProtocolEncoding::Default => {
                let mut data = Vec::with_capacity(6); data.extend_from_slice(b"\x1b[M");
                data.push((code + 32).min(255) as u8); data.push((x + 32).min(255) as u8); data.push((y + 32).min(255) as u8); Some(data)
            }
        }
    }
}

fn push_utf8_code(data: &mut Vec<u8>, value: u16) {
    let scalar = char::from_u32(u32::from(value) + 32).unwrap_or(' ');
    let mut buf = [0u8; 4]; data.extend_from_slice(scalar.encode_utf8(&mut buf).as_bytes());
}
fn effective_color(color: Color, bold: bool) -> Color { match (color, bold) { (Color::Default, false) => Color::Rgb(215,222,232), (Color::Default,true) => Color::Rgb(100,255,218), (value,_) => value } }
fn escape_markup(value: &str, output: &mut String) { for ch in value.chars() { match ch { '&' => output.push_str("&amp;"), '<' => output.push_str("&lt;"), '>' => output.push_str("&gt;"), _ => output.push(ch) } } }
fn append_span(output: &mut String, color: Option<Color>, bold: bool, italic: bool, text: &str) {
    let Some(color) = color else { output.push_str(text); return; }; let color = color_hex(color);
    if bold && italic { output.push_str(&format!("<b><i><font color=\"{}\">{}</font></i></b>",color,text)); }
    else if bold { output.push_str(&format!("<b><font color=\"{}\">{}</font></b>",color,text)); }
    else if italic { output.push_str(&format!("<i><font color=\"{}\">{}</font></i>",color,text)); }
    else { output.push_str(&format!("<font color=\"{}\">{}</font>",color,text)); }
}
fn color_hex(color: Color) -> String { match color { Color::Default => "#d7dee8".into(), Color::Idx(index) => { let rgb=ansi_index_rgb(index); format!("#{:02x}{:02x}{:02x}",rgb.0,rgb.1,rgb.2) }, Color::Rgb(r,g,b) => format!("#{:02x}{:02x}{:02x}",r,g,b) } }
fn ansi_index_rgb(index: u8) -> (u8,u8,u8) {
    const BASIC:[(u8,u8,u8);16]=[(0,0,0),(205,49,49),(13,188,121),(229,229,16),(36,114,200),(188,63,188),(17,168,205),(229,229,229),(102,102,102),(241,76,76),(35,209,139),(245,245,67),(59,142,234),(214,112,214),(41,184,219),(255,255,255)];
    if index<16{return BASIC[index as usize]}; if (16..=231).contains(&index){let n=index-16;let r=(n/36)%6;let g=(n/6)%6;let b=n%6;let level=|v:u8|if v==0{0}else{55+40*v};return(level(r),level(g),level(b));} let gray=8+10*(index-232);(gray,gray,gray)
}
