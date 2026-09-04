use std::path::Path;
use std::sync::Arc;

use russh::client::{self, Handle, Handler};
use russh::keys::{self, load_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, CryptoVec};
use tokio::sync::mpsc;

struct ClientHandler;

impl Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, _server_public_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        // Temporary bootstrap behavior. NAGX host-key storage/verification comes next.
        Ok(true)
    }
}

#[derive(Clone)]
pub struct PtySession {
    tx: mpsc::UnboundedSender<PtyCommand>,
}

enum PtyCommand {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16, pixel_width: u16, pixel_height: u16 },
}

impl PtySession {
    pub fn send(&self, data: Vec<u8>) -> Result<(), String> {
        self.tx.send(PtyCommand::Input(data)).map_err(|_| "PTY input channel closed".to_string())
    }

    pub fn resize(&self, cols: u16, rows: u16, pixel_width: u16, pixel_height: u16) -> Result<(), String> {
        self.tx
            .send(PtyCommand::Resize { cols, rows, pixel_width, pixel_height })
            .map_err(|_| "PTY resize channel closed".to_string())
    }
}

struct SessionChannelHandler {
    output_tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl Handler for SessionChannelHandler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, _server_public_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

async fn connect_authenticated(
    host: &str,
    port: u16,
    username: &str,
    auth_mode: &str,
    secret: String,
    passphrase: String,
) -> Result<Handle<ClientHandler>, String> {
    let config = client::Config::default();
    let config = Arc::new(config);
    let mut handle = client::connect(config, (host, port), ClientHandler)
        .await
        .map_err(|err| format!("SSH connection failed: {err}"))?;

    match auth_mode {
        "password" => {
            let authenticated = handle
                .authenticate_password(username, secret)
                .await
                .map_err(|err| format!("SSH password authentication failed: {err}"))?;
            if !authenticated { return Err("SSH password authentication rejected".to_string()); }
        }
        "key" => {
            let key_path = Path::new(&secret);
            let key_password = if passphrase.is_empty() { None } else { Some(passphrase.as_str()) };
            let key = load_secret_key(key_path, key_password)
                .map_err(|err| format!("Could not load SSH private key: {err}"))?;
            let key = PrivateKeyWithHashAlg::new(
                key,
                if key.key_type() == keys::key::KeyPairType::RSA { Some(keys::HashAlg::Sha2_512) } else { None },
            );
            let authenticated = handle
                .authenticate_publickey(username, key)
                .await
                .map_err(|err| format!("SSH public-key authentication failed: {err}"))?;
            if !authenticated { return Err("SSH public-key authentication rejected".to_string()); }
        }
        other => return Err(format!("Unsupported authentication mode: {other}")),
    }

    Ok(handle)
}

pub async fn connect_password(host: String, port: u16, username: String, password: String) -> Result<Handle<ClientHandler>, String> {
    connect_authenticated(&host, port, &username, "password", password, String::new()).await
}

pub async fn connect_key(host: String, port: u16, username: String, key_path: String, passphrase: String) -> Result<Handle<ClientHandler>, String> {
    connect_authenticated(&host, port, &username, "key", key_path, passphrase).await
}

pub async fn connect_pty(
    host: String,
    port: u16,
    username: String,
    auth_mode: String,
    secret: String,
    passphrase: String,
) -> Result<(PtySession, mpsc::UnboundedReceiver<Vec<u8>>), String> {
    let handle = connect_authenticated(
        &host,
        port,
        &username,
        auth_mode.as_str(),
        secret,
        passphrase,
    )
    .await?;

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|err| format!("Could not open SSH session: {err}"))?;

    channel
        .request_pty(false, "xterm-256color", 120, 32, 0, 0, &[])
        .await
        .map_err(|err| format!("Could not request SSH PTY: {err}"))?;

    channel
        .shell(true)
        .await
        .map_err(|err| format!("Could not start SSH shell: {err}"))?;

    let (tx, mut command_rx) = mpsc::unbounded_channel();
    let (output_tx, output_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(command) = command_rx.recv() => {
                    match command {
                        PtyCommand::Input(data) => {
                            let _ = channel.data(CryptoVec::from(data)).await;
                        }
                        PtyCommand::Resize { cols, rows, pixel_width, pixel_height } => {
                            let _ = channel.window_change(cols, rows, pixel_width, pixel_height).await;
                        }
                    }
                }
                message = channel.wait() => {
                    match message {
                        Some(ChannelMsg::Data(data)) => {
                            let _ = output_tx.send(data.to_vec());
                        }
                        Some(ChannelMsg::ExtendedData(_, data)) => {
                            let _ = output_tx.send(data.to_vec());
                        }
                        Some(ChannelMsg::Eof) | None => break,
                        _ => {}
                    }
                }
            }
        }
    });

    Ok((PtySession { tx }, output_rx))
}
