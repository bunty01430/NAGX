#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerIdentity {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceSnapshot {
    pub cpu_percent: u8,
    pub memory_percent: u8,
    pub disk_percent: u8,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub gpu_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceState {
    pub name: String,
    pub active: bool,
    pub status: String,
}
