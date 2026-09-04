use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::PublicKeyOrCertificate;
use russh::{ChannelMsg, Disconnect};
use tokio::sync::mpsc::{self, Receiver, Sender};

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

pub async fn connect_password(
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<client::Handle<NagxHandler>, String> {
    let config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        keepalive_interval: Some(Duration::from_secs(10)),
        ..Default::default()
    };

    let mut handle = client::connect(
        Arc::new(config),
        (host.as_str(), port),
        NagxHandler,
    )
    .await
    .map_err(|err| format!("SSH connection failed: {err}"))?;

    let auth = handle
        .authenticate_password(username, password)
        .await
        .map_err(|err| format!("SSH authentication failed: {err}"))?;

    if !auth.success() {
        return Err("SSH authentication rejected by server".to_string());
    }

    Ok(handle)
}

pub async fn connect_pty(
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<(PtySession, Receiver<Vec<u8>>), String> {
    let mut handle = connect_password(host, port, username, password).await?;

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
            tokio::select! {
                command = command_rx.recv() => {
                    match command {
                        Some(PtyCommand::Input(data)) => {
                            if channel.data_bytes(data).await.is_err() {
                                break;
                            }
                        }
                        Some(PtyCommand::Resize { cols, rows, pixel_width, pixel_height }) => {
                            if channel
                                .window_change(
                                    u32::from(cols),
                                    u32::from(rows),
                                    u32::from(pixel_width),
                                    u32::from(pixel_height),
                                )
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                message = channel.wait() => {
                    match message {
                        Some(ChannelMsg::Data { data }) => {
                            if output_tx.send(data.to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Some(ChannelMsg::ExtendedData { data, .. }) => {
                            if output_tx.send(data.to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                        _ => {}
                    }
                }
            }
        }

        let _ = handle
            .disconnect(Disconnect::ByApplication, "NAGX session closed", "English")
            .await;
    });

    Ok((PtySession { commands: command_tx }, output_rx))
}
