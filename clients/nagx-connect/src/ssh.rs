use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::timeout;

pub struct NagxHandler;

impl Handler for NagxHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Temporary bootstrap behavior. NAGX host-key storage/verification comes next.
        Ok(true)
    }
}

#[derive(Debug)]
enum PtyCommand {
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    },
}

#[derive(Clone)]
pub struct PtySession {
    commands: Sender<PtyCommand>,
}

impl PtySession {
    pub fn send(&self, data: Vec<u8>) -> Result<(), String> {
        self.commands
            .try_send(PtyCommand::Input(data))
            .map_err(|err| format!("PTY input queue error: {err}"))
    }

    pub fn resize(
        &self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<(), String> {
        self.commands
            .try_send(PtyCommand::Resize {
                cols,
                rows,
                pixel_width,
                pixel_height,
            })
            .map_err(|err| format!("PTY resize queue error: {err}"))
    }
}

fn default_username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

async fn connect_authenticated(
    host: &str,
    port: u16,
    username: String,
    auth_mode: &str,
    secret: String,
    passphrase: String,
) -> Result<client::Handle<NagxHandler>, String> {
    let config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        keepalive_interval: Some(Duration::from_secs(10)),
        ..Default::default()
    };

    let mut handle = client::connect(
        Arc::new(config),
        (host, port),
        NagxHandler,
    )
    .await
    .map_err(|err| format!("SSH connection failed: {err}"))?;

    match auth_mode {
        "password" => {
            if username.trim().is_empty() {
                return Err("SSH username is required for password authentication".to_string());
            }
            if secret.trim().is_empty() {
                return Err("SSH password is required for password authentication".to_string());
            }
            let auth = handle
                .authenticate_password(username, secret)
                .await
                .map_err(|err| format!("SSH password authentication failed: {err}"))?;
            if !auth.success() {
                return Err("SSH password authentication rejected".to_string());
            }
        }
        "key" => {
            let key_path = Path::new(&secret);
            if !key_path.is_file() {
                return Err(format!("SSH private key not found: {}", key_path.display()));
            }

            let password = if passphrase.is_empty() { None } else { Some(passphrase.as_str()) };
            let key_pair = load_secret_key(key_path, password)
                .map_err(|err| format!("Could not load SSH private key: {err}"))?;

            let rsa_hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(|err| format!("Could not negotiate RSA hash: {err}"))?
                .flatten();

            let key_username = if username.trim().is_empty() {
                default_username()
            } else {
                username
            };
            if key_username.trim().is_empty() {
                return Err("SSH username is empty and no local username is available. Enter a username.".to_string());
            }

            let auth = handle
                .authenticate_publickey(
                    key_username,
                    PrivateKeyWithHashAlg::new(Arc::new(key_pair), rsa_hash),
                )
                .await
                .map_err(|err| format!("SSH public-key authentication failed: {err}"))?;

            if !auth.success() {
                return Err("SSH public-key authentication rejected".to_string());
            }
        }
        other => return Err(format!("Unsupported SSH authentication mode: {other}")),
    }

    Ok(handle)
}

pub async fn connect_password(
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<client::Handle<NagxHandler>, String> {
    connect_authenticated(&host, port, username, "password", password, String::new()).await
}

pub async fn connect_key(
    host: String,
    port: u16,
    username: String,
    key_path: String,
    passphrase: String,
) -> Result<client::Handle<NagxHandler>, String> {
    connect_authenticated(&host, port, username, "key", key_path, passphrase).await
}

pub async fn connect_pty(
    host: String,
    port: u16,
    username: String,
    auth_mode: String,
    secret: String,
    passphrase: String,
) -> Result<(PtySession, Receiver<Vec<u8>>), String> {
    // The native UI keeps the four-string connection callback stable. Key mode is
    // represented as "NAGXKEY:<path>\n<passphrase>" in the secret argument.
    let (actual_mode, actual_secret, actual_passphrase) = if auth_mode == "password" && secret.starts_with("NAGXKEY:") {
        let payload = &secret[8..];
        let mut parts = payload.splitn(2, '\n');
        let key_path = parts.next().unwrap_or_default().to_string();
        let key_passphrase = parts.next().unwrap_or_default().to_string();
        ("key".to_string(), key_path, key_passphrase)
    } else {
        (auth_mode, secret, passphrase)
    };

    let mut handle = connect_authenticated(
        &host,
        port,
        username,
        actual_mode.as_str(),
        actual_secret,
        actual_passphrase,
    )
    .await?;

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|err| format!("Could not open SSH session: {err}"))?;

    channel
        .request_pty(false, "xterm-256color", 120, 32, 0, 0, &[])
        .await
        .map_err(|err| format!("PTY request failed: {err}"))?;

    channel
        .request_shell(true)
        .await
        .map_err(|err| format!("Shell request failed: {err}"))?;

    let (command_tx, mut command_rx) = mpsc::channel::<PtyCommand>(256);
    let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(256);

    tokio::spawn(async move {
        loop {
            match timeout(Duration::from_millis(20), channel.wait()).await {
                Ok(Some(ChannelMsg::Data { data })) => {
                    if output_tx.send(data.to_vec()).await.is_err() {
                        break;
                    }
                }
                Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                    if output_tx.send(data.to_vec()).await.is_err() {
                        break;
                    }
                }
                Ok(Some(ChannelMsg::Eof)) | Ok(Some(ChannelMsg::Close)) | Ok(None) => break,
                Ok(Some(_)) | Err(_) => {}
            }

            while let Ok(command) = command_rx.try_recv() {
                let result = match command {
                    PtyCommand::Input(data) => channel.data_bytes(data).await,
                    PtyCommand::Resize {
                        cols,
                        rows,
                        pixel_width,
                        pixel_height,
                    } => channel
                        .window_change(
                            u32::from(cols),
                            u32::from(rows),
                            u32::from(pixel_width),
                            u32::from(pixel_height),
                        )
                        .await,
                };

                if result.is_err() {
                    let _ = output_tx.send(b"\r\n[NAGX] PTY transport error\r\n".to_vec()).await;
                    break;
                }
            }

            if command_rx.is_closed() {
                break;
            }
        }

        let _ = handle
            .disconnect(Disconnect::ByApplication, "NAGX session closed", "English")
            .await;
    });

    Ok((PtySession { commands: command_tx }, output_rx))
}
