mod ssh;
mod terminal_emulator;

slint::include_modules!();

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use rfd::FileDialog;
use slint::{Model, ModelRc, Timer, TimerMode, VecModel};
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

#[derive(Clone)]
struct TerminalUiSnapshot {
    markup: String,
    plain: String,
    cursor_x: u16,
    cursor_y: u16,
    cursor_visible: bool,
    mouse_reporting: bool,
}

type PtyPool = Arc<Mutex<Vec<Option<PtySession>>>>;
type EmulatorPool = Arc<Mutex<Vec<TerminalEmulator>>>;
type ProfilePool = Arc<Mutex<Vec<Option<ConnectionProfile>>>>;
type SnapshotPool = Arc<Mutex<Vec<Option<TerminalUiSnapshot>>>>;

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
        let mut prefixed = vec![0x1b];
        prefixed.extend(data);
        prefixed
    } else { data }
}

fn build_terminal_data(id: usize, name: String, state: &str, host: &str, user: &str, focused: bool) -> TerminalData {
    TerminalData {
        id: id as i32,
        name: name.into(),
        state: state.into(),
        host: host.into(),
        user: user.into(),
        shell: "bash".into(),
        content: slint::StyledText::from_plain_text("NAGX terminal offline."),
        plain: "NAGX terminal offline.".into(),
        cursor_x: 0,
        cursor_y: 0,
        cursor_visible: false,
        mouse_reporting: false,
        focused,
    }
}

fn update_terminal_model(model: &Rc<VecModel<TerminalData>>, index: usize, data: TerminalData) {
    if index < model.row_count() {
        model.set_row_data(index, data);
    } else {
        while model.row_count() < index {
            let filler = model.row_count();
            model.push(build_terminal_data(filler, format!("terminal-{:02}", filler + 1), "DISCONNECTED", "", "", false));
        }
        model.push(data);
    }
}

fn snapshot_terminal(emulators: &EmulatorPool, index: usize) -> Option<TerminalUiSnapshot> {
    let emulators = emulators.lock().ok()?;
    let terminal = emulators.get(index)?;
    let (cursor_y, cursor_x) = terminal.cursor_position();
    Some(TerminalUiSnapshot {
        markup: terminal.render_markup(),
        plain: terminal.render(),
        cursor_x,
        cursor_y,
        cursor_visible: terminal.cursor_visible(),
        mouse_reporting: terminal.mouse_reporting_enabled(),
    })
}

fn apply_snapshot_to_model(model: &Rc<VecModel<TerminalData>>, snapshots: &SnapshotPool, index: usize) {
    let snapshot = snapshots.lock().ok().and_then(|mut pool| pool.get_mut(index).and_then(Option::take));
    let Some(snapshot) = snapshot else { return; };
    let data = model.row_data(index).unwrap_or_else(|| build_terminal_data(index, format!("terminal-{:02}", index + 1), "CONNECTED", "", "", false));
    let mut updated = data;
    updated.content = slint::StyledText::from_markdown(&snapshot.markup).unwrap_or_else(|_| slint::StyledText::from_plain_text(&snapshot.markup));
    updated.plain = snapshot.plain.into();
    updated.cursor_x = i32::from(snapshot.cursor_x);
    updated.cursor_y = i32::from(snapshot.cursor_y);
    updated.cursor_visible = snapshot.cursor_visible;
    updated.mouse_reporting = snapshot.mouse_reporting;
    update_terminal_model(model, index, updated);
}

fn paste_to_terminal(emulators: &EmulatorPool, ptys: &PtyPool, index: usize) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    let text = clipboard.get_text().map_err(|e| format!("Clipboard read failed: {e}"))?;
    if text.is_empty() { return Ok(()); }
    let bracketed = emulators.lock().map_err(|_| "terminal mutex poisoned".to_string())?.get(index).map(|t| t.bracketed_paste_enabled()).unwrap_or(false);
    let data = if bracketed {
        let mut wrapped = Vec::with_capacity(text.len() + 12);
        wrapped.extend_from_slice(b"\x1b[200~");
        wrapped.extend_from_slice(text.as_bytes());
        wrapped.extend_from_slice(b"\x1b[201~");
        wrapped
    } else { text.into_bytes() };
    let pty = ptys.lock().map_err(|_| "PTY mutex poisoned".to_string())?.get(index).and_then(|p| p.clone()).ok_or_else(|| "Terminal session is not connected".to_string())?;
    pty.send(data)
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let model = Rc::new(VecModel::from(vec![
        build_terminal_data(0, "terminal-01".to_string(), "DISCONNECTED", "", "", true),
    ]));
    window.set_terminals(ModelRc::from(Rc::clone(&model)));

    let ptys: PtyPool = Arc::new(Mutex::new(vec![None]));
    let emulators: EmulatorPool = Arc::new(Mutex::new(vec![TerminalEmulator::new(32, 120, 10_000)]));
    let profiles: ProfilePool = Arc::new(Mutex::new(vec![None]));
    let snapshots: SnapshotPool = Arc::new(Mutex::new(vec![None]));

    let ui_timer = Timer::default();
    {
        let model = Rc::clone(&model);
        let snapshots = Arc::clone(&snapshots);
        ui_timer.start(TimerMode::Repeated, Duration::from_millis(33), move || {
            let indices: Vec<usize> = snapshots.lock().map(|pool| pool.iter().enumerate().filter_map(|(i, value)| value.as_ref().map(|_| i)).collect()).unwrap_or_default();
            for index in indices {
                apply_snapshot_to_model(&model, &snapshots, index);
            }
        });
    }

    {
        let weak = window.as_weak();
        window.on_browse_key(move || {
            if let Some(path) = FileDialog::new().set_title("Select SSH private key").add_filter("SSH private keys", &["pem", "ppk", "key"]).add_filter("All files", &["*"]).pick_file() {
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
        let snapshots = Arc::clone(&snapshots);
        let model = Rc::clone(&model);
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

            {
                let mut ptys = ptys.lock().expect("PTY mutex poisoned");
                while ptys.len() <= index { ptys.push(None); }
            }
            {
                let mut emulators = emulators.lock().expect("terminal mutex poisoned");
                while emulators.len() <= index { emulators.push(TerminalEmulator::new(32, 120, 10_000)); }
            }
            {
                let mut profiles = profiles.lock().expect("profile mutex poisoned");
                while profiles.len() <= index { profiles.push(None); }
                profiles[index] = Some(ConnectionProfile { host: host.to_string(), username: username.to_string(), auth_mode: mode.clone(), secret: secret.clone(), passphrase: passphrase.clone() });
            }
            {
                let mut snapshots = snapshots.lock().expect("snapshot mutex poisoned");
                while snapshots.len() <= index { snapshots.push(None); }
            }

            {
                let mut data = model.row_data(index).unwrap_or_else(|| build_terminal_data(index, format!("terminal-{:02}", index + 1), "DISCONNECTED", "", "", false));
                data.state = "CONNECTING".into();
                data.host = host.to_string().into();
                data.user = username.to_string().into();
                data.focused = true;
                update_terminal_model(&model, index, data);
            }
            if let Some(window) = weak.upgrade() {
                window.set_active_terminal_index(index as i32);
                window.set_status_text(format!("TERMINAL {:02} · CONNECTING", index + 1).into());
                window.set_server_text(format!("{}@{}:22", username, host).into());
            }

            let weak_for_thread = weak.clone();
            let ptys_thread = Arc::clone(&ptys);
            let emulators_thread = Arc::clone(&emulators);
            let snapshots_thread = Arc::clone(&snapshots);
            thread::spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        let _ = weak_for_thread.upgrade_in_event_loop(move |window| window.set_status_text(format!("TERMINAL {:02} · RUNTIME ERROR: {err}", index + 1).into()));
                        return;
                    }
                };
                match runtime.block_on(ssh::connect_pty(host.to_string(), 22, username.to_string(), mode.clone(), secret.clone(), passphrase.clone())) {
                    Ok((pty, mut output_rx)) => {
                        if let Ok(mut slots) = ptys_thread.lock() {
                            if index >= slots.len() { slots.resize_with(index + 1, || None); }
                            slots[index] = Some(pty);
                        }
                        if let Ok(mut slots) = emulators_thread.lock() {
                            if index >= slots.len() { slots.resize_with(index + 1, || TerminalEmulator::new(32, 120, 10_000)); }
                            slots[index] = TerminalEmulator::new(32, 120, 10_000);
                        }
                        let host_ui = host.to_string();
                        let username_ui = username.to_string();
                        let _ = weak_for_thread.upgrade_in_event_loop(move |window| {
                            window.set_server_text(format!("{}@{}:22", username_ui, host_ui).into());
                            window.set_status_text(format!("TERMINAL {:02} · CONNECTED / ANSI PTY", index + 1).into());
                            window.invoke_focus_input();
                        });

                        runtime.block_on(async move {
                            while let Some(bytes) = output_rx.recv().await {
                                let mut emulator = emulators_thread.lock().expect("terminal mutex poisoned");
                                if index >= emulator.len() {
                                    continue;
                                }
                                let terminal = &mut emulator[index];
                                let markup = terminal.process(&bytes);
                                let plain = terminal.render();
                                let (cursor_y, cursor_x) = terminal.cursor_position();
                                let snapshot = TerminalUiSnapshot {
                                    markup,
                                    plain,
                                    cursor_x,
                                    cursor_y,
                                    cursor_visible: terminal.cursor_visible(),
                                    mouse_reporting: terminal.mouse_reporting_enabled(),
                                };
                                drop(emulator);
                                if let Ok(mut pool) = snapshots_thread.lock() {
                                    if index >= pool.len() { pool.resize_with(index + 1, || None); }
                                    pool[index] = Some(snapshot);
                                }
                            }
                            if let Ok(mut slots) = ptys_thread.lock() { if index < slots.len() { slots[index] = None; } }
                            let _ = weak_for_thread.upgrade_in_event_loop(move |window| {
                                if index as i32 == window.get_active_terminal_index() { window.set_status_text(format!("TERMINAL {:02} · PTY DISCONNECTED", index + 1).into()); }
                            });
                        });
                    }
                    Err(err) => {
                        let _ = weak_for_thread.upgrade_in_event_loop(move |window| window.set_status_text(format!("TERMINAL {:02} · {err}", index + 1).into()));
                    }
                }
            });
        });
    }

    {
        let weak = window.as_weak();
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        let profiles = Arc::clone(&profiles);
        let snapshots = Arc::clone(&snapshots);
        let model = Rc::clone(&model);
        window.on_new_terminal(move || {
            let index = model.row_count();
            model.push(build_terminal_data(index, format!("terminal-{:02}", index + 1), "DISCONNECTED", "", "", true));
            if let Ok(mut ptys) = ptys.lock() { ptys.push(None); }
            if let Ok(mut emulators) = emulators.lock() { emulators.push(TerminalEmulator::new(32, 120, 10_000)); }
            if let Ok(mut profiles) = profiles.lock() { profiles.push(None); }
            if let Ok(mut snapshots) = snapshots.lock() { snapshots.push(None); }
            if let Some(window) = weak.upgrade() {
                window.set_active_terminal_index(index as i32);
                window.set_connection_open(true);
                window.set_status_text(format!("TERMINAL {:02} CREATED", index + 1).into());
            }
        });
    }

    {
        let weak = window.as_weak();
        let model = Rc::clone(&model);
        window.on_focus_terminal(move |index| {
            let index = index.max(0) as usize;
            if index >= model.row_count() { return; }
            if let Some(window) = weak.upgrade() { window.set_active_terminal_index(index as i32); }
            for i in 0..model.row_count() {
                if let Some(mut data) = model.row_data(i) {
                    data.focused = i == index;
                    model.set_row_data(i, data);
                }
            }
        });
    }

    {
        let model = Rc::clone(&model);
        window.on_close_terminal(move |index| {
            let index = index.max(0) as usize;
            if index >= model.row_count() { return; }
            if let Some(data) = model.row_data(index) {
                let host = data.host.to_string();
                let user = data.user.to_string();
                model.set_row_data(index, build_terminal_data(index, data.name.to_string(), "CLOSED", host.as_str(), user.as_str(), false));
            }
        });
    }

    {
        let model = Rc::clone(&model);
        window.on_split_terminal(move |index| {
            let index = index.max(0) as usize;
            let next = model.row_count();
            let source = model.row_data(index).unwrap_or_else(|| build_terminal_data(index, format!("terminal-{:02}", index + 1), "DISCONNECTED", "", "", false));
            let host = source.host.to_string();
            let user = source.user.to_string();
            model.push(build_terminal_data(next, format!("terminal-{:02}", next + 1), source.state.to_string().as_str(), host.as_str(), user.as_str(), true));
        });
    }

    {
        let weak = window.as_weak();
        let ptys_for_keyboard = Arc::clone(&ptys);
        window.on_terminal_key(move |index, text, control, alt, _shift| {
            let index = index.max(0) as usize;
            let bytes = terminal_key_bytes(text.as_str(), control, alt);
            if bytes.is_empty() { return; }
            if let Ok(ptys) = ptys_for_keyboard.lock() {
                if let Some(pty) = ptys.get(index).and_then(|p| p.clone()) { let _ = pty.send(bytes); }
            }
            if let Some(window) = weak.upgrade() { window.set_active_terminal_index(index as i32); }
        });
    }

    {
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        window.on_terminal_resize(move |index, cols, rows, pixel_width, pixel_height| {
            let index = index.max(0) as usize;
            let cols = cols.clamp(1, 400) as u16;
            let rows = rows.clamp(1, 200) as u16;
            let pixel_width = pixel_width.clamp(1, u16::MAX as i32) as u16;
            let pixel_height = pixel_height.clamp(1, u16::MAX as i32) as u16;
            if let Ok(mut emulators) = emulators.lock() {
                while emulators.len() <= index { emulators.push(TerminalEmulator::new(32, 120, 10_000)); }
                if emulators[index].size() != (rows, cols) { emulators[index].set_size(rows, cols); }
            }
            if let Ok(ptys) = ptys.lock() {
                if let Some(pty) = ptys.get(index).and_then(|p| p.clone()) { let _ = pty.resize(cols, rows, pixel_width, pixel_height); }
            }
        });
    }

    {
        let ptys = Arc::clone(&ptys);
        let emulators = Arc::clone(&emulators);
        window.on_terminal_mouse(move |index, button, kind, x, y, shift, alt, control| {
            let index = index.max(0) as usize;
            let bytes = emulators.lock().expect("terminal mutex poisoned").get(index)
                .and_then(|terminal| terminal.mouse_report(button.clamp(0, 5) as u8, kind.clamp(0, 3) as u8, x.clamp(1, 1000) as u16, y.clamp(1, 1000) as u16, shift, alt, control));
            if let Some(bytes) = bytes {
                if let Ok(ptys) = ptys.lock() {
                    if let Some(pty) = ptys.get(index).and_then(|p| p.clone()) { let _ = pty.send(bytes); }
                }
            }
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

    window.on_focus_input(move || {});
    let _ui_timer = ui_timer;
    window.run()
}
