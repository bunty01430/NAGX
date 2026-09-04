mod ssh;

slint::include_modules!();

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let active_connection: Arc<Mutex<Option<russh::client::Handle<ssh::NagxHandler>>>> =
        Arc::new(Mutex::new(None));

    let weak_window = window.as_weak();
    let connections = Arc::clone(&active_connection);
    window.on_connect(move |host, username, password| {
        let weak_window = weak_window.clone();
        let connections = Arc::clone(&connections);

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

            let result = runtime.block_on(ssh::connect_password(
                host,
                22,
                username,
                password,
            ));

            match result {
                Ok(handle) => {
                    *connections.lock().expect("connection mutex poisoned") = Some(handle);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = weak_window.upgrade() {
                            window.set_status_text("CONNECTED".into());
                        }
                    });

                    loop {
                        thread::sleep(Duration::from_secs(60));
                    }
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

    window.run()
}
