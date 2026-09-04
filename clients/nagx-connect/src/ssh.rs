use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::PublicKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{self, Receiver, Sender};

pub struct NagxHandler;

impl Handler for NagxHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _server_public_key: &PublicKey) -> Result<bool, Self::Error> {
        // Temporary bootstrap behavior. NAGX host-key storage/verification comes next.
        Ok(true)
    }
}

#[derive(Clone)]
pub struct PtySession {
    input: Sender<Vec<u8>>,
}

impl PtySession {
    pub async fn send(&self, data: Vec<u8>) -> Result<(), String> {
        self.input
            .send(data)
            .await
            .map_err(|_| "PTY input channel closed".to_string())
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

    let channel = handle
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

    let stream = channel.into_stream();
    let (mut reader, mut writer) = tokio::io::split(stream);

    let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(256);
    let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(256);

    tokio::spawn(async move {
        let mut buf = vec![0_u8; 8192];

        loop {
            tokio::select! {
                input = input_rx.recv() => {
                    match input {
                        Some(data) => {
                            if writer.write_all(&data).await.is_err() {
                                break;
                            }
                            if writer.flush().await.is_err() {
                                break;
                            }
                        }
                        None => {
                            let _ = writer.shutdown().await;
                            break;
                        }
                    }
                }
                read_result = reader.read(&mut buf) => {
                    match read_result {
                        Ok(0) => break,
                        Ok(count) => {
                            if output_tx.send(buf[..count].to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        }

        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "NAGX session closed", "English")
            .await;
    });

    Ok((PtySession { input: input_tx }, output_rx))
}
