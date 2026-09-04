use nagx_core::constants::{APP_NAME, PROTOCOL_MAJOR, PROTOCOL_NAME};

fn main() {
    println!("{APP_NAME} server daemon");
    println!("protocol: {PROTOCOL_NAME}/{PROTOCOL_MAJOR}");
    println!("transport: SSH channel (planned)");
    println!("pty: real OS PTY (planned)");
}
