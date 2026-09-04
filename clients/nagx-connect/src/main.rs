mod ssh;
mod terminal_emulator;

slint::include_modules!();

use std::sync::{Arc, Mutex};
use std::thread;

use arboard::Clipboard;
use rfd::FileDialog;
use ssh::PtySession;
use terminal_emulator::TerminalEmulator;

const TERMINAL_SLOTS: usize = 3;

type PtySlots = Arc<Mutex<[Option<PtySession>; TERMINAL_SLOTS]>>;
type TerminalSlots = Arc<Mutex<[TerminalEmulator; TERMINAL_SLOTS]>>;

fn terminal_key_bytes(text: &str, control: bool, alt: bool) -> Vec<u8> {
    use slint::platform::Key;

    let data = if Key::Return.into() == text {
        vec![b'\r']
    } else if Key::Backspace.into() == text {
        vec![0x7f]
    } else if Key::Tab.into() == text {
        vec![b'\t']
    } else if Key::Escape.into() == text {
        vec![0x1b]
    } else if Key::UpArrow.into() == text {
        b"\x1b[A".to_vec()
    } else if Key::DownArrow.into() == text {
        b"\x1b[B".to_vec()
    } else if Key::RightArrow.into() == text {
        b"\x1b[C".to_vec()
    } else if Key::LeftArrow.into() == text {
        b"\x1b[D".to_vec()
    } else if Key::Delete.into() == text {
        b"\x1b[3~".to_vec()
    } else if Key::Home.into() == text {
        b"\x1b[H".to_vec()
    } else if Key::End.into() == text {
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
        prefixed
    } else {
        data
    }
}

fn terminal_snapshot(
    terminals: &TerminalSlots,
    slot: usize,
) -> Result<(String, String, (u16, u16), bool, bool), String> {
    let slots = terminals.lock().map_err(|_| "terminal mutex poisoned".to_string())?;
    let terminal = &slots[slot];
    Ok((
        terminal.render_markup(),
        terminal.render(),
        terminal.cursor_position(),
        terminal.cursor_visible(),
        terminal.mouse_reporting_enabled(),
    ))
}

fn apply_active_terminal_snapshot(window: &MainWindow, terminals: &TerminalSlots, slot: usize) {
    let Ok((markup, plain, (cursor_y, cursor_x), cursor_visible, mouse_reporting)) =
        terminal_snapshot(terminals, slot)
    else {
        return;
    };

    let styled = slint::StyledText::from_markdown(&markup)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(&markup));

    match slot {
        0 => {
            window.set_terminal_1_styled(styled);
            window.set_terminal_1_plain(plain.into());
            window.set_terminal_1_cursor_x(i32::from(cursor_x));
            window.set_terminal_1_cursor_y(i32::from(cursor_y));
            window.set_terminal_1_cursor_visible(cursor_visible);
            window.set_terminal_1_mouse_reporting(mouse_reporting);
        }
        1 => {
            window.set_terminal_2_styled(styled);
            window.set_terminal_2_plain(plain.into());
            window.set_terminal_2_cursor_x(i32::from(cursor_x));
            window.set_terminal_2_cursor_y(i32::from(cursor_y));
            window.set_terminal_2_cursor_visible(cursor_visible);
            window.set_terminal_2_mouse_reporting(mouse_reporting);
        }
        2 => {
            window.set_terminal_3_styled(styled);
            window.set_terminal_3_plain(plain.into());
            window.set_terminal_3_cursor_x(i32::from(cursor_x));
            window.set_terminal_3_cursor_y(i32::from(cursor_y));
            window.set_terminal_3_cursor_visible(cursor_visible);
            window.set_terminal_3_mouse_reporting(mouse_reporting);
        }
        _ => {}
    }
}

fn paste_from_clipboard(
    terminals: &TerminalSlots,
    ptys: &PtySlots,
    slot: usize,
) -> Result<(), String> {
    let mut clipboard = Clipboard::new()
        .map_err(|err| format!("Clipboard unavailable: {err}"))?;
    let text = clipboard
        .get_text()
        .map_err(|err| format!("Clipboard read failed: {err}"))?;
    if text.is_empty() || slot >= TERMINAL_SLOTS {
        return Ok(());
    }

    let bracketed = terminals
        .lock()
        .map_err(|_| "terminal mutex poisoned".to_string())?[slot]
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
    secret: String,
) {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                let message = format!("T{} RUNTIME ERROR: {err}", slot + 1);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak_window.upgrade() {
                        window.set_status_text(message.into());
                    }
                });
                return;
            }
        };

        match runtime.block_on(ssh::connect_pty(
            host.clone(),
            22,
            username.clone(),
            "password".to_string(),
            secret,
            String::new(),
        )) {
            Ok((pty, mut output_rx)) => {
                if let Ok(mut slots) = ptys.lock() {
                    slots[slot] = Some(pty);
                }
                if let Ok(mut slots) = terminals.lock() {
                    slots[slot] = TerminalEmulator::new(32, 120, 2000);
                }

                let terminals_for_ui = Arc::clone(&terminals);
                let weak_for_ui = weak_window.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak_for_ui.upgrade() {
                        window.set_server_text(
                            format!("T{} · {}@{}:22", slot + 1, username, host).into(),
                        );
                        window.set_status_text(
                            format!("T{} CONNECTED / ANSI PTY", slot + 1).into(),
                        );
                        match slot {
                            0 => window.set_terminal_1_state("CONNECTED".into()),
                            1 => window.set_terminal_2_state("CONNECTED".into()),
                            2 => window.set_terminal_3_state("CONNECTED".into()),
                            _ => {}
                        }
                        if slot == (window.get_active_terminal() as usize).saturating_sub(1) {
                            apply_active_terminal_snapshot(&window, &terminals_for_ui, slot);
                            window.invoke_focus_terminal();
                        }
                    }
                });

                runtime.block_on(async move {
                    while let Some(bytes) = output_rx.recv().await {
                        let (
                            markup,
                            plain,
                            (cursor_y, cursor_x),
                            cursor_visible,
                            mouse_reporting,
                        ) = {
                            let mut slots = terminals
                                .lock()
                                .expect("terminal mutex poisoned");
                            let terminal = &mut slots[slot];
                            let markup = terminal.process(&bytes);
                            let plain = terminal.render();
                            let position = terminal.cursor_position();
                            (
                                markup,
                                plain,
                                position,
                                terminal.cursor_visible(),
                                terminal.mouse_reporting_enabled(),
                            )
                        };

                        let weak_for_output = weak_window.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = weak_for_output.upgrade() {
                                let styled = slint::StyledText::from_markdown(&markup)
                                    .unwrap_or_else(|_| slint::StyledText::from_plain_text(&markup));
                                match slot {
                                    0 => {
                                        window.set_terminal_1_styled(styled);
                                        window.set_terminal_1_plain(plain.into());
                                        window.set_terminal_1_cursor_x(i32::from(cursor_x));
                                        window.set_terminal_1_cursor_y(i32::from(cursor_y));
                                        window.set_terminal_1_cursor_visible(cursor_visible);
                                        window.set_terminal_1_mouse_reporting(mouse_reporting);
                                    }
                                    1 => {
                                        window.set_terminal_2_styled(styled);
                                        window.set_terminal_2_plain(plain.into());
                                        window.set_terminal_2_cursor_x(i32::from(cursor_x));
                                        window.set_terminal_2_cursor_y(i32::from(cursor_y));
                                        window.set_terminal_2_cursor_visible(cursor_visible);
                                        window.set_terminal_2_mouse_reporting(mouse_reporting);
                                    }
                                    2 => {
                                        window.set_terminal_3_styled(styled);
                                        window.set_terminal_3_plain(plain.into());
                                        window.set_terminal_3_cursor_x(i32::from(cursor_x));
                                        window.set_terminal_3_cursor_y(i32::from(cursor_y));
                                        window.set_terminal_3_cursor_visible(cursor_visible);
                                        window.set_terminal_3_mouse_reporting(mouse_reporting);
                                    }
                                    _ => {}
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
                                0 => {
                                    window.set_terminal_1_state("OFFLINE".into());
                                    window.set_terminal_1_cursor_visible(false);
                                    window.set_terminal_1_mouse_reporting(false);
                                }
                                1 => {
                                    window.set_terminal_2_state("OFFLINE".into());
                                    window.set_terminal_2_cursor_visible(false);
                                    window.set_terminal_2_mouse_reporting(false);
                                }
                                2 => {
                                    window.set_terminal_3_state("OFFLINE".into());
                                    window.set_terminal_3_cursor_visible(false);
                                    window.set_terminal_3_mouse_reporting(false);
                                }
                                _ => {}
                            }
                            if slot == (window.get_active_terminal() as usize).saturating_sub(1) {
                                window.set_status_text(
                                    format!("T{} PTY DISCONNECTED", slot + 1).into(),
                                );
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

    let weak_for_browse = window.as_weak();
    window.on_browse_key(move || {
        if let Some(path) = FileDialog::new()
            .set_title("Select SSH private key")
            .add_filter("SSH private keys", &["pem", "ppk", "key"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            if let Some(window) = weak_for_browse.upgrade() {
                window.set_key_path(path.to_string_lossy().to_string().into());
                window.set_status_text("PRIVATE KEY SELECTED".into());
            }
        }
    });

    let weak_for_connect = window.as_weak();
    let ptys_for_connect = Arc::clone(&ptys);
    let terminals_for_connect = Arc::clone(&terminals);
    window.on_connect_slot(move |slot, host, username, password| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        if host.trim().is_empty() || username.trim().is_empty() {
            if let Some(window) = weak_for_connect.upgrade() {
                window.set_status_text("INVALID CONNECTION".into());
            }
            return;
        }

        if let Some(window) = weak_for_connect.upgrade() {
            match slot {
                0 => window.set_terminal_1_state("CONNECTING".into()),
                1 => window.set_terminal_2_state("CONNECTING".into()),
                2 => window.set_terminal_3_state("CONNECTING".into()),
                _ => {}
            }
            window.set_active_terminal((slot + 1) as i32);
            apply_active_terminal_snapshot(&window, &terminals_for_connect, slot);
            window.invoke_focus_terminal();
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
    });

    let weak_for_select = window.as_weak();
    let terminals_for_select = Arc::clone(&terminals);
    window.on_select_terminal(move |slot| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        if let Some(window) = weak_for_select.upgrade() {
            apply_active_terminal_snapshot(&window, &terminals_for_select, slot);
            window.set_status_text(format!("T{} SELECTED", slot + 1).into());
            window.set_server_text(format!("T{} · active", slot + 1).into());
        }
    });

    let ptys_for_keyboard = Arc::clone(&ptys);
    window.on_terminal_key(move |slot, text, control, alt, _shift| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        let bytes = terminal_key_bytes(text.as_str(), control, alt);
        if bytes.is_empty() {
            return;
        }
        if let Some(pty) = ptys_for_keyboard
            .lock()
            .expect("PTY mutex poisoned")[slot]
            .clone()
        {
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
            let mut terminals = terminals_for_resize
                .lock()
                .expect("terminal mutex poisoned");
            if terminals[slot].size() != (rows, cols) {
                terminals[slot].set_size(rows, cols);
            }
        }

        if let Some(pty) = ptys_for_resize
            .lock()
            .expect("PTY mutex poisoned")[slot]
            .clone()
        {
            let _ = pty.resize(cols, rows, pixel_width, pixel_height);
        }
    });

    let ptys_for_mouse = Arc::clone(&ptys);
    let terminals_for_mouse = Arc::clone(&terminals);
    window.on_terminal_mouse(move |slot, button, kind, x, y, shift, alt, control| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        let bytes = terminals_for_mouse
            .lock()
            .expect("terminal mutex poisoned")[slot]
            .mouse_report(
                button.clamp(0, 5) as u8,
                kind.clamp(0, 3) as u8,
                x.clamp(1, 1000) as u16,
                y.clamp(1, 1000) as u16,
                shift,
                alt,
                control,
            );
        if let Some(bytes) = bytes {
            if let Some(pty) = ptys_for_mouse
                .lock()
                .expect("PTY mutex poisoned")[slot]
                .clone()
            {
                let _ = pty.send(bytes);
            }
        }
    });

    let ptys_for_wheel = Arc::clone(&ptys);
    let terminals_for_wheel = Arc::clone(&terminals);
    window.on_terminal_wheel(move |slot, delta_y| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        let button = if delta_y < 0 { 5 } else { 4 };
        if let Some(bytes) = terminals_for_wheel
            .lock()
            .expect("terminal mutex poisoned")[slot]
            .mouse_report(button, 1, 1, 1, false, false, false)
        {
            if let Some(pty) = ptys_for_wheel
                .lock()
                .expect("PTY mutex poisoned")[slot]
                .clone()
            {
                let _ = pty.send(bytes);
            }
        }
    });

    let ptys_for_paste = Arc::clone(&ptys);
    let terminals_for_paste = Arc::clone(&terminals);
    window.on_terminal_paste(move |slot| {
        let slot = (slot.clamp(1, 3) - 1) as usize;
        if let Err(err) = paste_from_clipboard(&terminals_for_paste, &ptys_for_paste, slot) {
            eprintln!("[NAGX] {err}");
        }
    });

    window.run()
}
