mod ssh;
mod terminal_emulator;

slint::include_modules!();

use std::sync::{Arc, Mutex};
use std::thread;

use ssh::PtySession;
use terminal_emulator::TerminalEmulator;

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

    if control && data.len() == 1 && data[0].is_ascii_alphabetic() {
        return vec![data[0].to_ascii_lowercase() - b'a' + 1];
    }

    if alt && !data.is_empty() {
        let mut prefixed = vec![0x1b];
        prefixed.extend(data);
        return prefixed;
    }

    data
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let active_pty: Arc<Mutex<Option<PtySession>>> = Arc::new(Mutex::new(None));
    let terminal = Arc::new(Mutex::new(TerminalEmulator::new(32, 120, 2000)));

    let weak_for_connect = window.as_weak();
    let pty_for_connect = Arc::clone(&active_pty);
    let terminal_for_connect = Arc::clone(&terminal);
    window.on_connect(move |host, username, password| {
        let weak_window = weak_for_connect.clone();
        let pty_for_connect = Arc::clone(&pty_for_connect);
        let terminal_for_connect = Arc::clone(&terminal_for_connect);

        if host.trim().is_empty() || username.trim().is_empty() {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = weak_window.upgrade() {
                    window.set_status_text("INVALID CONNECTION".into());
                }
            });
            return;
        }

        let _ = slint::invoke_from_event_loop({
            let weak_window = weak_window.clone();
            move || {
                if let Some(window) = weak_window.upgrade() {
                    window.set_status_text("CONNECTING...".into());
                    window.set_server_text(format!("{}@{}:22", username, host).into());
                }
            }
        });

        thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let message = format!("RUNTIME ERROR: {err}");
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak_window.upgrade() {
                            window.set_status_text(message.into());
                        }
                    });
                    return;
                }
            };

            match runtime.block_on(ssh::connect_pty(host, 22, username, password)) {
                Ok((pty, mut output_rx)) => {
                    *pty_for_connect.lock().expect("PTY mutex poisoned") = Some(pty.clone());

                    {
                        let mut terminal = terminal_for_connect.lock().expect("terminal mutex poisoned");
                        *terminal = TerminalEmulator::new(32, 120, 2000);
                    }

                    let _ = slint::invoke_from_event_loop({
                        let weak_window = weak_window.clone();
                        move || {
                            if let Some(window) = weak_window.upgrade() {
                                window.set_status_text("CONNECTED / VT100 PTY".into());
                                window.set_terminal_text("NAGX terminal initializing...\n".into());
                                window.invoke_focus_terminal();
                            }
                        }
                    });

                    runtime.block_on(async move {
                        while let Some(bytes) = output_rx.recv().await {
                            let rendered = {
                                let mut terminal = terminal_for_connect
                                    .lock()
                                    .expect("terminal mutex poisoned");
                                terminal.process(&bytes)
                            };

                            let _ = slint::invoke_from_event_loop({
                                let weak_window = weak_window.clone();
                                move || {
                                    if let Some(window) = weak_window.upgrade() {
                                        window.set_terminal_text(rendered.into());
                                    }
                                }
                            });
                        }

                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = weak_window.upgrade() {
                                window.set_status_text("PTY DISCONNECTED".into());
                            }
                        });
                    });

                    *pty_for_connect.lock().expect("PTY mutex poisoned") = None;
                }
                Err(err) => {
                    let message = format!("ERROR: {err}");
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak_window.upgrade() {
                            window.set_status_text(message.into());
                        }
                    });
                }
            }
        });
    });

    let pty_for_keyboard = Arc::clone(&active_pty);
    window.on_terminal_key(move |text, control, alt| {
        let bytes = terminal_key_bytes(text.as_str(), control, alt);
        if bytes.is_empty() {
            return;
        }

        let pty = pty_for_keyboard
            .lock()
            .expect("PTY mutex poisoned")
            .clone();

        if let Some(pty) = pty {
            let _ = pty.send(bytes);
        }
    });

    window.run()
}
