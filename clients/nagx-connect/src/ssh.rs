use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::PublicKey;

pub struct NagxHandler;

impl Handler for NagxHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _server_public_key: &PublicKey) -> Result<bool, Self::Error> {
        // Temporary bootstrap behavior. NAGX host-key storage/verification comes next.
        Ok(true)
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
