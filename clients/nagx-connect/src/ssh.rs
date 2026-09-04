use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handler};
use russh::{client::Handle, ChannelMsg};
use russh::keys::PublicKey;

struct NagxHandler;

#[async_trait::async_trait]
impl Handler for NagxHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _server_public_key: &PublicKey) -> Result<bool, Self::Error> {
        // Host-key verification will be replaced with NAGX known-hosts management.
        Ok(true)
    }
}

pub async fn connect_password(
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<Handle<NagxHandler>, String> {
    let config = client::Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        ..Default::default()
    };

    let mut handle = client::connect(Arc::new(config), (host.as_str(), port), NagxHandler)
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

#[allow(dead_code)]
p async fn open_shell(handle: &mut Handle<NagxHandler>) -> Result<(), String> {
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

    Ok(())
}

#[allow(dead_code)]
p async fn read_message(channel: &mut russh::Channel<Msg>) -> Option<ChannelMsg> {
    channel.wait().await
}
