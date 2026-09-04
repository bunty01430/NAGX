mod ssh;
mod terminal_emulator;

slint::include_modules!();

use std::sync::{Arc, Mutex};
use std::thread;

use arboard::Clipboard;
use ssh::PtySession;
use terminal_emulator::TerminalEmulator;

const TERMINAL_SLOTS: usize = 3;

type PtySlots = Arc<Mutex<[Option<PtySession>; TERMINAL_SLOTS]>>;
type TerminalSlots = Arc<Mutex<[TerminalEmulator; TERMINAL_SLOTS]>>;

fn terminal_key_bytes(text: &str, control: bool, alt: bool) -> Vec<u8> {
    use slint::platform::Key;

    let return_key: slint::SharedString = Key::Return.into();
    let backspace_key: slint::SharedString = Key::Backspace.into();
    let tab_key: slint::SharedString = Key::Tab.into();
    let escape_key: slint::SharedString = Key::Escape.into();
    let up_key: slint::SharedString = Key::UpArrow.into();
    let down_key: slint::SharedString = Key::DownArrow.into();
    let right_key: slint::SharedString = Key::RightArrow.into();
    let left_key: slint::SharedString = Key::LeftArrow.into();
    let delete_key: slint::SharedString = Key::Delete.into();
    let home_key: slint::SharedString = Key::Home.into();
    let end_key: slint::SharedString = Key::End.into();

    let data = if text == return_key {
        vec![b'\r']
    } else if text == backspace_key {
        vec![0x7f]
    } else if text == tab_key {
        vec![b'\t']
    } else if text == escape_key {
        vec![0x1b]
    } else if text == up_key {
        b"\x1b[A".to_vec()
    } else if text == down_key {
        b"\x1b[B".to_vec()
    } else if text == right_key {
        b"\x1b[C".to_vec()
    } else if text == left_key {
        b"\x1b[D".to_vec()
    } else if text == delete_key {
        b"\x1b[3~".to_vec()
    } else if text == home_key {
        b"\x1b[H".to_vec()
    } else if text == end_key {
        b"\x1b[F".to_vec()
    } else {
        text.as_bytes().to_vec()
    };

    let data = if control && data.len() == 1 && data[0].is_ascii_alphabetic() {
        vec![data[0].to_ascii_lowercase() - b'a' + 1]
    } else {
        data
    };

    if alt && !data.is_empty() {
        let mut prefixed = vec![0x1b];
        prefixed.extend(data);
        return prefixed;
    }

    data
}

fn paste_from_clipboard(terminal: &Arc<Mutex<TerminalSlots>>, ptys: &PtySlots, slot: usize) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|err| format!("Clipboard unavailable: {err}"))?;
    let text = clipboard.get_text().map_err(|err| format!("Clipboard read failed: {err}"))?;
    if text.is_empty() || slot >= TERMINAL_SLOTS { return Ok(()); }

    let bracketed = terminal
        .lock()
        .map_err(|_| "Terminal mutex poisoned".to_string())?[slot]
        .bracketed_paste_enabled();

    let data = if bracketed {
        let mut wrapped = Vec::with_capacity(text.len() + 12);
        wrapped.extend_from_slice(b"\x1b[200~");
        wrapped.extend_from_slice(text.as_bytes());
        wrapped.extend_from_slice(b"\x1b[201~");
        wrapped
    } else {
        text.into_bytes()
    };

    let pty = ptys
        .lock()
        .map_err(|_| "PTY mutex poisoned".to_string())?[slot]
        .clone()
        .ok_or_else(|| "Terminal session is not connected".to_string())?;
    pty.send(data)
}

fn spawn_terminal_connection(
    weak_window: slint::Weak<MainWindow>,
    ptys: PtySlots,
    terminals: TerminalSlots,
    slot: usize,
    host: String,
    username: String,
    password: String,
) {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(err) => {
                let message = format!("T{} RUNTIME ERROR: {err}", slot + 1);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak_window.upgrade() { window.set_status_text(message.into()); }
                });
                return;
            }
        };

        match runtime.block_on(ssh::connect_pty(host.clone(), 22, username.clone(), password)) {
            Ok((pty, mut output_rx)) => {
                if let Ok(mut slots) = ptys.lock() {
                    slots[slot] = Some(pty.clone());
                }
                if let Ok(mut slots) = terminals.lock() {
                    slots[slot] = TerminalEmulator::new(32, 120, 2000);
                }

                let _ = slint::invoke_from_event_loop({
                    let weak_window = weak_window.clone();
                    move || {
                        if let Some(window) = weak_window.upgrade() {
                            window.set_server_text(format!("T{} · {}@{}:22", slot + 1, username, host).into());
                            window.set_status_text(format!("T{} CONNECTED / ANSI PTY", slot + 1).into());
                            if slot == window.get_active_terminal() as usize {
                                window.set_cursor_visible(true);
                                window.invoke_focus_terminal();
                            }
                            match slot {
                                0 => window.set_terminal_1_state("CONNECTED".into()),
                                1 => window.set_terminal_2_state("CONNECTED".into()),
                                2 => window.set_terminal_3_state("CONNECTED".into()),
                                _ => {}
                            }
                        }
                    }
                });

                runtime.block_on(async move {
                    while let Some(bytes) = output_rx.recv().await {
                        let (markup, plain, (cursor_y, cursor_x), cursor_visible, mouse_reporting) = {
                            let mut slots = terminals.lock().expect("terminal mutex poisoned");
                            let terminal = &mut slots[slot];
                            let markup = terminal.process(&bytes);
                            let plain = terminal.render();
                            let position = terminal.cursor_position();
                            (markup, plain, position, terminal.cursor_visible(), terminal.mouse_reporting_enabled())
                        };

                        let _ = slint::invoke_from_event_loop({
                            let weak_window = weak_window.clone();
                            move || {
                                if let Some(window) = weak_window.upgrade() {
                                    match slot {
                                        0 => {
                                            window.set_terminal_1_styled(
                                                slint::StyledText::from_markdown(&markup)
                                                    .unwrap_or_else(|_| slint::StyledText::from_plain_text(&markup)),
                                            );
                                            window.set_terminal_1_plain(plain.into());
                                            window.set_terminal_1_cursor_x(i32::from(cursor_x));
                                            window.set_terminal_1_cursor_y(i32::from(cursor_y));
                                            window.set_terminal_1_cursor_visible(cursor_visible);
                                            window.set_terminal_1_mouse_reporting(mouse_reporting);
                                        }
                                        1 => {
                                            window.set_terminal_2_styled(
                                                slint::StyledText::from_markdown(&markup)
                                                    .unwrap_or_else(|_| slint::StyledText::from_plain_text(&markup)),
                                            );
                                            window.set_terminal_2_plain(plain.into());
                                            window.set_terminal_2_cursor_x(i32::from(cursor_x));
                                            window.set_terminal_2_cursor_y(i32::from(cursor_y));
                                            window.set_terminal_2_cursor_visible(cursor_visible);
                                            window.set_terminal_2_mouse_reporting(mouse_reporting);
                                        }
                                        2 => {
                                            window.set_terminal_3_styled(
                                                slint::StyledText::from_markdown(&markup)
                                                    .unwrap_or_else(|_| slint::StyledText::from_plain_text(&markup)),
                                            );
                                            window.set_terminal_3_plain(plain.into());
                                            window.set_terminal_3_cursor_x(i32::from(cursor_x));
                                            window.set_terminal_3_cursor_y(i32::from(cursor_y));
                                            window.set_terminal_3_cursor_visible(cursor_visible);
                                            window.set_terminal_3_mouse_reporting(mouse_reporting);
                                        }
                                        _ => {}
                                    }
                                    if slot == window.get_active_terminal() as usize {
                                        window.set_cursor_x(i32::from(cursor_x));
                                        window.set_cursor_y(i32::from(cursor_y));
                                        window.set_cursor_visible(cursor_visible);
                                        window.set_mouse_reporting(mouse_reporting);
                                    }
                                }
                            }
                        });
                    }

                    if let Ok(mut slots) = ptys.lock() {
                        slots[slot] = None;
                    }

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak_window.upgrade() {
                            match slot {
                                0 => { window.set_terminal_1_state("OFFLINE".into()); window.set_terminal_1_cursor_visible(false); window.set_terminal_1_mouse_reporting(false); }
                                1 => { window.set_terminal_2_state("OFFLINE".into()); window.set_terminal_2_cursor_visible(false); window.set_terminal_2_mouse_reporting(false); }
                                2 => { window.set_terminal_3_state("OFFLINE".into()); window.set_terminal_3_cursor_visible(false); window.set_terminal_3_mouse_reporting(false); }
                                _ => {}
                            }
                            if slot == window.get_active_terminal() as usize {
                                window.set_status_text(format!("T{} PTY DISCONNECTED", slot + 1).into());
                                window.set_cursor_visible(false);
                                window.set_mouse_reporting(false);
                            }
                        }
                    });
                });
            }
            Err(err) => {
                let message = format!("T{} ERROR: {err}", slot + 1);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak_window.upgrade() {
                        match slot {
                            0 => window.set_terminal_1_state("ERROR".into()),
                            1 => window.set_terminal_2_state("ERROR".into()),
                            2 => window.set_terminal_3_state("ERROR".into()),
                            _ => {}
                        }
                        window.set_status_text(message.into());
                    }
                });
            }
        }
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let ptys: PtySlots = Arc::new(Mutex::new([None, None, None]));
    let terminals: TerminalSlots = Arc::new(Mutex::new([
        TerminalEmulator::new(32, 120, 2000),
        TerminalEmulator::new(32, 120, 2000),
        TerminalEmulator::new(32, 120, 2000),
    ]));

    let weak_for_connect = window.as_weak();
    let ptys_for_connect = Arc::clone(&ptys);
    let terminals_for_connect = Arc::clone(&terminals);
    window.on_connect_slot(move |slot, host, username, password| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        if host.trim().is_empty() || username.trim().is_empty() {
            weak_for_connect.upgrade().map(|window| window.set_status_text("INVALID CONNECTION".into()));
            return;
        }
        spawn_terminal_connection(
            weak_for_connect.clone(),
            Arc::clone(&ptys_for_connect),
            Arc::clone(&terminals_for_connect),
            slot,
            host.to_string(),
            username.to_string(),
            password.to_string(),
        );
        match slot {
            0 => window.set_terminal_1_state("CONNECTING".into()),
            1 => window.set_terminal_2_state("CONNECTING".into()),
            2 => window.set_terminal_3_state("CONNECTING".into()),
            _ => {}
        }
        window.set_active_terminal((slot + 1) as i32);
    });

    let ptys_for_keyboard = Arc::clone(&ptys);
    window.on_terminal_key(move |slot, text, control, alt, _shift| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        let bytes = terminal_key_bytes(text.as_str(), control, alt);
        if bytes.is_empty() { return; }
        if let Some(pty) = ptys_for_keyboard.lock().expect("PTY mutex poisoned")[slot].clone() {
            let _ = pty.send(bytes);
        }
    });

    let ptys_for_resize = Arc::clone(&ptys);
    let terminals_for_resize = Arc::clone(&terminals);
    window.on_terminal_resize(move |slot, cols, rows, pixel_width, pixel_height| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        let cols = cols.clamp(1, 400) as u16;
        let rows = rows.clamp(1, 200) as u16;
        let pixel_width = pixel_width.clamp(1, u16::MAX as i32) as u16;
        let pixel_height = pixel_height.clamp(1, u16::MAX as i32) as u16;

        {
            let mut terminals = terminals_for_resize.lock().expect("terminal mutex poisoned");
            if terminals[slot].size() != (rows, cols) { terminals[slot].set_size(rows, cols); }
        }
        if let Some(pty) = ptys_for_resize.lock().expect("PTY mutex poisoned")[slot].clone() {
            let _ = pty.resize(cols, rows, pixel_width, pixel_height);
        }
    });

    let ptys_for_mouse = Arc::clone(&ptys);
    let terminals_for_mouse = Arc::clone(&terminals);
    window.on_terminal_mouse(move |slot, button, kind, x, y, shift, alt, control| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        let bytes = terminals_for_mouse
            .lock().expect("terminal mutex poisoned")[slot]
            .mouse_report(button.clamp(0, 5) as u8, kind.clamp(0, 3) as u8, x.clamp(1, 1000) as u16, y.clamp(1, 1000) as u16, shift, alt, control);
        if let Some(bytes) = bytes {
            if let Some(pty) = ptys_for_mouse.lock().expect("PTY mutex poisoned")[slot].clone() { let _ = pty.send(bytes); }
        }
    });

    let ptys_for_wheel = Arc::clone(&ptys);
    let terminals_for_wheel = Arc::clone(&terminals);
    window.on_terminal_wheel(move |slot, delta_y| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        let button = if delta_y < 0 { 5 } else { 4 };
        if let Some(bytes) = terminals_for_wheel.lock().expect("terminal mutex poisoned")[slot].mouse_report(button, 1, 1, 1, false, false, false) {
            if let Some(pty) = ptys_for_wheel.lock().expect("PTY mutex poisoned")[slot].clone() { let _ = pty.send(bytes); }
        }
    });

    let ptys_for_paste = Arc::clone(&ptys);
    let terminals_for_paste = Arc::clone(&terminals);
    window.on_terminal_paste(move |slot| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        if let Err(err) = paste_from_clipboard(&terminals_for_paste, &ptys_for_paste, slot) { eprintln!("[NAGX] {err}"); }
    });

    window.run()
}
