use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::net::IpAddr;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub service_name: String,

    pub client_ip: IpAddr,

    pub client_port: u16,

    pub server_ip: IpAddr,

    pub server_port: u16,

    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub client_timeout: Duration,

    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub server_timeout: Duration,

    pub tls_enabled: bool,

    #[serde(default)]
    pub tls_cert_file: Option<String>,

    #[serde(default)]
    pub tls_key_file: Option<String>,

    #[serde(default)]
    pub tls_ca_file: Option<String>,

    #[serde(default, rename = "script_path")]
    pub python_script: Option<String>,
}

fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

impl Config {
    pub fn load_from_file(path: &str) -> anyhow::Result<Config> {
        let reader = BufReader::new(File::open(path)?);
        Ok(serde_yaml::from_reader(reader)?)
    }
}
