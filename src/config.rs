use byte_unit::{Byte, Unit};
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub service_name: String,

    #[serde(alias = "from_ip")]
    pub client_ip: IpAddr,

    #[serde(alias = "from_port")]
    pub client_port: u16,

    #[serde(alias = "to_ip")]
    pub server_ip: IpAddr,

    #[serde(alias = "to_port")]
    pub server_port: u16,

    #[serde(
        alias = "from_timeout",
        default = "default_timeout",
        with = "humantime_serde"
    )]
    pub client_timeout: Duration,

    #[serde(
        alias = "to_timeout",
        default = "default_timeout",
        with = "humantime_serde"
    )]
    pub server_timeout: Duration,

    #[serde(alias = "from_max_history", default = "default_max_history")]
    pub client_max_history: Byte,

    #[serde(alias = "to_max_history", default = "default_max_history")]
    pub server_max_history: Byte,

    pub tls_enabled: bool,

    #[serde(default)]
    pub tls_cert_file: Option<String>,

    #[serde(default)]
    pub tls_key_file: Option<String>,

    #[serde(default)]
    pub tls_ca_file: Option<String>,

    #[serde(default, rename = "script_path")]
    pub python_script: Option<String>,

    pub dump_enabled: bool,

    pub dump_path: Option<PathBuf>,

    pub dump_format: Option<String>,

    #[serde(with = "humantime_serde")]
    pub dump_interval: Option<Duration>,

    #[serde(default = "default_max_packets")]
    pub dump_max_packets: usize,
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_max_history() -> Byte {
    Byte::from_u64_with_unit(512, Unit::MiB).unwrap()
}

fn default_max_packets() -> usize {
    512
}

impl Config {
    pub fn load_from_file(path: &str) -> anyhow::Result<Config> {
        let reader = BufReader::new(File::open(path)?);
        Ok(serde_yaml::from_reader(reader)?)
    }
}
