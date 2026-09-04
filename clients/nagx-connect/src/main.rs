mod ssh;

slint::include_modules!();

use std::sync::{Arc, Mutex};
use std::thread;

use ssh::PtySession;

fn terminal_key_bytes(text: &str, control: bool, alt: bool) -> Vec<u8> {
    let mut data = match text {
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::Return) => vec![b'\r'],
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::Backspace) => vec![0x7f],
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::Tab) => vec![b'\t'],
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::Escape) => vec![0x1b],
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::UpArrow) => b"\x1b[A".to_vec(),
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::DownArrow) => b"\x1b[B".to_vec(),
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::RightArrow) => b"\x1b[C".to_vec(),
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::LeftArrow) => b"\x1b[D".to_vec(),
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::Delete) => b"\x1b[3~".to_vec(),
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::Home) => b"\x1b[H".to_vec(),
        value if value == <slint::platform::Key as Into<slint::SharedString>>::into(slint::platform::Key::End) => b"\x1b[F".to_vec(),
        value => value.as_bytes().to_vec(),
    };

    if control && data.len() == 1 && data[0].is_ascii_alphabetic() {
        data[0] = data[0].to_ascii_lowercase() - b'a' + 1;
        return data;
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

    let weak_for_connect = window.as_weak();
    let pty_for_connect = Arc::clone(&active_pty);
    window.on_connect(move |host, username, password| {
        let weak_window = weak_for_connect.clone();
        let pty_for_connect = Arc::clone(&pty_for_connect);

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

            let result = runtime.block_on(ssh::connect_pty(
                host,
                22,
                username,
                password,
            ));

            match result {
                Ok((pty, mut output_rx)) => {
                    *pty_for_connect.lock().expect("PTY mutex poisoned") = Some(pty.clone());

                    let _ = slint::invoke_from_event_loop({
                        let weak_window = weak_window.clone();
                        move || {
                            if let Some(window) = weak_window.upgrade() {
                                window.set_status_text("CONNECTED / PTY".into());
                                window.set_terminal_text("NAGX PTY connected.\n".into());
                                window.invoke_terminal_focus();
                            }
                        }
                    });

                    let weak_output = weak_window.clone();
                    runtime.block_on(async move {
                        let mut terminal_text = String::new();

                        while let Some(bytes) = output_rx.recv().await {
                            let chunk = String::from_utf8_lossy(&bytes);
                            terminal_text.push_str(&chunk);

                            if terminal_text.len() > 64 * 1024 {
                                let keep_from = terminal_text.len() - 48 * 1024;
                                terminal_text.drain(..keep_from);
                            }

                            let text = terminal_text.clone();
                            let _ = slint::invoke_from_event_loop({
                                let weak_output = weak_output.clone();
                                move || {
                                    if let Some(window) = weak_output.upgrade() {
                                        window.set_terminal_text(text.into());
                                    }
                                }
                            });
                        }

                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = weak_output.upgrade() {
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
            thread::spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    let _ = runtime.block_on(pty.send(bytes));
                }
            });
        }
    });

    window.run()
}
