use nagx_config::ServerConfig;
use nagx_core::constants::APP_NAME;
use nagx_session::session::SessionState;
use nagx_terminal::terminal::TerminalSize;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let host = args.first().cloned().unwrap_or_else(|| "localhost".into());
    let user = args.get(1).cloned().unwrap_or_else(|| "root".into());
    let config = ServerConfig::new(host.clone(), user.clone());
    let session = SessionState::new("local-bootstrap", format!("{}@{}:{}", user, host, config.port));
    let size = TerminalSize::default();

    println!("{APP_NAME} Connect");
    println!("server : {}", session.server);
    println!("terminal: {}x{}", size.cols, size.rows);
    println!("status : ready");
    println!();
    println!("Native GUI + SSH transport + real PTY are the next runtime layer.");
}
