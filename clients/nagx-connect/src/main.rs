mod ssh;
mod terminal_emulator;

slint::include_modules!();

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use arboard::Clipboard;
use rfd::FileDialog;
use slint::{Model, ModelRc, VecModel};
use ssh::PtySession;
use terminal_emulator::TerminalEmulator;

#[derive(Clone)]
struct ConnectionProfile {
    host: String,
    username: String,
    auth_mode: String,
    secret: String,
    passphrase: String,
}

type PtyPool = Arc<Mutex<Vec<Option<PtySession>>>>;
type EmulatorPool = Arc<Mutex<Vec<TerminalEmulator>>>;
type ProfilePool = Arc<Mutex<Vec<Option<ConnectionProfile>>>>;

fn terminal_key_bytes(text: &str, control: bool, alt: bool) -> Vec<u8> {
    use slint::platform::Key;
    let key = |k: Key| -> slint::SharedString { k.into() };
    let data = if key(Key::Return) == text { vec![b'\r'] }
        else if key(Key::Backspace) == text { vec![0x7f] }
        else if key(Key::Tab) == text { vec![b'\t'] }
        else if key(Key::Escape) == text { vec![0x1b] }
        else if key(Key::UpArrow) == text { b"\x1b[A".to_vec() }
        else if key(Key::DownArrow) == text { b"\x1b[B".to_vec() }
        else if key(Key::RightArrow) == text { b"\x1b[C".to_vec() }
        else if key(Key::LeftArrow) == text { b"\x1b[D".to_vec() }
        else if key(Key::Delete) == text { b"\x1b[3~".to_vec() }
        else if key(Key::Home) == text { b"\x1b[H".to_vec() }
        else if key(Key::End) == text { b"\x1b[F".to_vec() }
        else { text.as_bytes().to_vec() };

    let data = if control && data.len() == 1 && data[0].is_ascii_alphabetic() {
        vec![data[0].to_ascii_lowercase() - b'a' + 1]
    } else { data };

    if alt && !data.is_empty() {
        let mut result = vec![0x1b];
        result.extend(data);
        result
    } else { data }
}

fn build_terminal_data(id: usize, name: String, state: &str, host: &str, user: &str, focused: bool) -> TerminalData {
    let message = "NAGX terminal ready. Connect this session to begin.";
    TerminalData {
        id: id as i32,
        name: name.into(),
        state: state.into(),
        host: host.into(),
        user: user.into(),
        shell: "bash".into(),
        content: slint::StyledText::from_plain_text(message),
        plain: message.into(),
        cursor_x: 0,
        cursor_y: 0,
        cursor_visible: false,
        mouse_reporting: false,
        focused,
    }
}

fn make_connected_data(
    current: TerminalData,
    markup: String,
    plain: String,
    cursor: (u16, u16),
    cursor_visible: bool,
    mouse_reporting: bool,
    state: &str,
) -> TerminalData {
    TerminalData {
        id: current.id,
        name: current.name,
        state: state.into(),
        host: current.host,
        user: current.user,
        shell: current.shell,
        content: slint::StyledText::from_markdown(&markup)
            .unwrap_or_else(|_| slint::StyledText::from_plain_text(&markup)),
        plain: plain.into(),
        cursor_x: i32::from(cursor.1),
        cursor_y: i32::from(cursor.0),
        cursor_visible,
        mouse_reporting,
        focused: current.focused,
    }
}

fn spawn_terminal(
    weak_window: slint::Weak<MainWindow>,
    ptys: PtyPool,
    emulators: EmulatorPool,
    index: usize,
    profile: ConnectionProfile,
) {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak_window.upgrade() { window.set_status_text(format!("RUNTIME ERROR: {err}").into()); }
                });
                return;
            }
        };

        match runtime.block_on(ssh::connect_pty(
            profile.host.clone(),
            22,
            profile.username.clone(),
            profile.auth_mode.clone(),
            profile.secret.clone(),
            profile.passphrase.clone(),
        )) {
            Ok((pty, mut output_rx)) => {
                if let Ok(mut pool) = ptys.lock() {
                    if index < pool.len() { pool[index] = Some(pty); }
                }
                if let Ok(mut pool) = emulators.lock() {
                    if index < pool.len() { pool[index] = TerminalEmulator::new(32, 120, 10_000); }
                }

                let host = profile.host.clone();
                let user = profile.username.clone();
                let weak = weak_window.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak.upgrade() {
                        let model = window.get_terminals();
                        if let Some(current) = model.row_data(index) {
                            let data = TerminalData {
                                state: "CONNECTED".into(),
                                host: host.clone().into(),
                                user: user.clone().into(),
                                ..current
                            };
                            model.set_row_data(index, data);
                        }
                        window.set_server_text(format!("{}@{}", user, host).into());
                        window.set_status_text("CONNECTED · SSH / PTY".into());
                    }
                });

                runtime.block_on(async move {
                    while let Some(bytes) = output_rx.recv().await {
                        let state = {
                            let mut pool = match emulators.lock() { Ok(pool) => pool, Err(_) => break };
                            let terminal = match pool.get_mut(index) { Some(terminal) => terminal, None => break };
                            let markup = terminal.process(&bytes);
                            let plain = terminal.render();
                            let cursor = terminal.cursor_position();
                            (markup, plain, cursor, terminal.cursor_visible(), terminal.mouse_reporting_enabled())
                        };

                        let weak = weak_window.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = weak.upgrade() {
                                let model = window.get_terminals();
                                if let Some(current) = model.row_data(index) {
                                    model.set_row_data(index, make_connected_data(current, state.0, state.1, state.2, state.3, state.4, "CONNECTED"));
                                }
                            }
                        });
                    }

                    if let Ok(mut pool) = ptys.lock() {
                        if index < pool.len() { pool[index] = None; }
                    }
                    let weak = weak_window.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak.upgrade() {
                            let model = window.get_terminals();
                            if let Some(current) = model.row_data(index) {
                                model.set_row_data(index, TerminalData { state: "OFFLINE".into(), cursor_visible: false, mouse_reporting: false, ..current });
                            }
                            window.set_status_text(format!("TERMINAL {:02} DISCONNECTED", index + 1).into());
                        }
                    });
                });
            }
            Err(err) => {
                let weak = weak_window.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = weak.upgrade() {
                        let model = window.get_terminals();
                        if let Some(current) = model.row_data(index) {
                            model.set_row_data(index, TerminalData { state: "ERROR".into(), cursor_visible: false, mouse_reporting: false, ..current });
                        }
                        window.set_status_text(format!("TERMINAL {:02} · {err}", index + 1).into());
                    }
                });
            }
        }
    });
}

fn paste_to_terminal(emulators: &EmulatorPool, ptys: &PtyPool, index: usize) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    let text = clipboard.get_text().map_err(|e| format!("Clipboard read failed: {e}"))?;
    if text.is_empty() { return Ok(()); }
    let bracketed = emulators.lock().map_err(|_| "terminal mutex poisoned".to_string())?
        .get(index).map(|t| t.bracketed_paste_enabled()).unwrap_or(false);
    let data = if bracketed {
        let mut wrapped = Vec::with_capacity(text.len() + 12);
        wrapped.extend_from_slice(b"\x1b[200~");
        wrapped.extend_from_slice(text.as_bytes());
        wrapped.extend_from_slice(b"\x1b[201~");
        wrapped
    } else { text.into_bytes() };
    let pty = ptys.lock().map_err(|_| "PTY mutex poisoned".to_string())?
        .get(index).and_then(|p| p.clone()).ok_or_else(|| "Terminal session is not connected".to_string())?;
    pty.send(data)
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let model: Rc<VecModel<TerminalData>> = Rc::new(VecModel::from_slice(&[
        build_terminal_data(0, "terminal-01".to_string(), "DISCONNECTED", "", "", true),
    ]));
    window.set_terminals(ModelRc::from(Rc::clone(&model)));

    let ptys: PtyPool = Arc::new(Mutex::new(vec![None]));
    let emulators: EmulatorPool = Arc::new(Mutex::new(vec![TerminalEmulator::new(32, 120, 10_000)]));
    let profiles: ProfilePool = Arc::new(Mutex::new(vec![None]));

    {
        let weak = window.as_weak();
        window.on_browse_key(move || {
            if let Some(path) = FileDialog::new()
                .set_title("Select SSH private key")
                .add_filter("SSH private keys", &["pem", "ppk", "key"])
                .add_filter("All files", &["*"])
                .pick_file()
            {
                if let Some(window) = weak.upgrade() {
                    window.set_key_path(path.to_string_lossy().to_string().into());
                    window.set_status_text("PRIVATE KEY SELECTED".into());
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        let profiles = Arc::clone(&profiles);
        window.on_connect_request(move |index, host, username, auth_mode, secret, passphrase| {
            let index = index.max(0) as usize;
            if host.trim().is_empty() {
                if let Some(window) = weak.upgrade() { window.set_status_text("HOST REQUIRED".into()); }
                return;
            }
            let (mode, secret, passphrase) = if auth_mode == 0 {
                if username.trim().is_empty() || secret.trim().is_empty() {
                    if let Some(window) = weak.upgrade() { window.set_status_text("USERNAME + PASSWORD REQUIRED".into()); }
                    return;
                }
                ("password".to_string(), secret.to_string(), String::new())
            } else {
                if secret.trim().is_empty() {
                    if let Some(window) = weak.upgrade() { window.set_status_text("PRIVATE KEY REQUIRED".into()); }
                    return;
                }
                ("key".to_string(), secret.to_string(), passphrase.to_string())
            };

            if let Ok(mut p) = ptys.lock() { if index >= p.len() { p.resize_with(index + 1, || None); } }
            if let Ok(mut e) = emulators.lock() { if index >= e.len() { e.resize_with(index + 1, || TerminalEmulator::new(32, 120, 10_000)); } }
            if let Ok(mut p) = profiles.lock() {
                if index >= p.len() { p.resize_with(index + 1, || None); }
                p[index] = Some(ConnectionProfile { host: host.to_string(), username: username.to_string(), auth_mode: mode.clone(), secret: secret.clone(), passphrase: passphrase.clone() });
            }
            if let Some(current) = model.row_data(index) {
                model.set_row_data(index, TerminalData { state: "CONNECTING".into(), host: host.clone().into(), user: username.clone().into(), ..current });
            }
            if let Some(window) = weak.upgrade() {
                window.set_active_terminal_index(index as i32);
                window.set_status_text(format!("CONNECTING TERMINAL {:02}", index + 1).into());
            }
            spawn_terminal(
                weak.clone(), Arc::clone(&ptys), Arc::clone(&emulators), index,
                ConnectionProfile { host: host.to_string(), username: username.to_string(), auth_mode: mode, secret, passphrase },
            );
        });
    }

    {
        let weak = window.as_weak();
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        let profiles = Arc::clone(&profiles);
        window.on_new_terminal(move || {
            let index = model.row_count();
            model.push(build_terminal_data(index, format!("terminal-{number:02}", number = index + 1), "DISCONNECTED", "", "", true));
            if let Ok(mut p) = ptys.lock() { p.push(None); }
            if let Ok(mut e) = emulators.lock() { e.push(TerminalEmulator::new(32, 120, 10_000)); }
            if let Ok(mut p) = profiles.lock() { p.push(None); }
            for i in 0..model.row_count() {
                if let Some(current) = model.row_data(i) { model.set_row_data(i, TerminalData { focused: i == index, ..current }); }
            }
            if let Some(window) = weak.upgrade() {
                window.set_active_terminal_index(index as i32);
                window.set_connection_open(true);
                window.set_status_text(format!("TERMINAL {:02} CREATED", index + 1).into());
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_focus_terminal(move |index| {
            let index = index.max(0) as usize;
            if index >= model.row_count() { return; }
            for i in 0..model.row_count() {
                if let Some(current) = model.row_data(i) { model.set_row_data(i, TerminalData { focused: i == index, ..current }); }
            }
            if let Some(window) = weak.upgrade() {
                window.set_active_terminal_index(index as i32);
                window.set_status_text(format!("TERMINAL {:02} FOCUSED", index + 1).into());
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_close_terminal(move |index| {
            let index = index.max(0) as usize;
            if model.row_count() <= 1 || index >= model.row_count() { return; }
            if let Ok(mut p) = ptys.lock() { if index < p.len() { p.remove(index); } }
            if let Ok(mut e) = emulators.lock() { if index < e.len() { e.remove(index); } }
            if let Ok(mut p) = profiles.lock() { if index < p.len() { p.remove(index); } }
            model.remove(index);
            let target = model.row_count().saturating_sub(1);
            for i in 0..model.row_count() {
                if let Some(current) = model.row_data(i) { model.set_row_data(i, TerminalData { focused: i == target, ..current }); }
            }
            if let Some(window) = weak.upgrade() {
                window.set_active_terminal_index(target as i32);
                window.set_status_text("TERMINAL CLOSED".into());
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_maximize_terminal(move |index| {
            if let Some(window) = weak.upgrade() { window.set_status_text(format!("WINDOW MANAGER · MAXIMIZE {:02}", index + 1).into()); }
        });
        let weak = window.as_weak();
        window.on_split_terminal(move |_index| {
            if let Some(window) = weak.upgrade() { window.invoke_new_terminal(); }
        });
    }

    {
        let ptys = Arc::clone(&ptys);
        window.on_terminal_key(move |index, text, control, alt, _shift| {
            let index = index.max(0) as usize;
            let data = terminal_key_bytes(text.as_str(), control, alt);
            if data.is_empty() { return; }
            if let Some(pty) = ptys.lock().ok().and_then(|p| p.get(index).and_then(|s| s.clone())) { let _ = pty.send(data); }
        });
    }

    {
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        window.on_terminal_paste(move |index| {
            let index = index.max(0) as usize;
            if let Err(err) = paste_to_terminal(&emulators, &ptys, index) { eprintln!("[NAGX] {err}"); }
        });
    }

    {
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        window.on_terminal_resize(move |index, cols, rows, pixel_width, pixel_height| {
            let index = index.max(0) as usize;
            let cols = cols.clamp(1, 400) as u16;
            let rows = rows.clamp(1, 200) as u16;
            let pixel_width = pixel_width.clamp(1, i32::from(u16::MAX)) as u16;
            let pixel_height = pixel_height.clamp(1, i32::from(u16::MAX)) as u16;
            if let Ok(mut pool) = emulators.lock() {
                if let Some(terminal) = pool.get_mut(index) {
                    if terminal.size() != (rows, cols) { terminal.set_size(rows, cols); }
                }
            }
            if let Some(pty) = ptys.lock().ok().and_then(|p| p.get(index).and_then(|s| s.clone())) { let _ = pty.resize(cols, rows, pixel_width, pixel_height); }
        });
    }

    {
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        window.on_terminal_mouse(move |index, button, kind, x, y, shift, alt, control| {
            let index = index.max(0) as usize;
            let bytes = emulators.lock().ok().and_then(|pool| pool.get(index).and_then(|t| t.mouse_report(button.clamp(0, 5) as u8, kind.clamp(0, 3) as u8, x.clamp(1, 1000) as u16, y.clamp(1, 1000) as u16, shift, alt, control)));
            if let Some(bytes) = bytes {
                if let Some(pty) = ptys.lock().ok().and_then(|p| p.get(index).and_then(|s| s.clone())) { let _ = pty.send(bytes); }
            }
        });
    }

    window.run()
}
