use anyhow::Context;
use chrono::Utc;
use etherparse::PacketBuilder;
use pcap_file::pcap::{PcapPacket, PcapWriter};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::RwLockWriteGuard;
use std::{fs::File, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use strfmt::{strfmt, strfmt_builder};
use tempfile::NamedTempFile;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::time::sleep_until;
use tokio::{fs, time::Instant};
use tracing::{error, info};

use crate::{config::Config, service::Service};

pub struct Dumper {
    path: PathBuf,
    format: String,
    interval: Duration,
    rx: mpsc::Receiver<DumperMsg>,
}

pub struct DumperMsg {
    src: SocketAddr,
    dst: SocketAddr,
    time: Duration,
    chunk: Vec<u8>,
}

pub type DumperChannel = mpsc::Sender<DumperMsg>;

impl Dumper {
    pub async fn start(service: &Service, config: &Config) -> anyhow::Result<DumperChannel> {
        let path = PathBuf::from(config.dump_path.clone().context("Dump path is required")?);

        match fs::metadata(&path).await {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    Err(anyhow::anyhow!("Expected a directory for pcaps"))?
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                tokio::fs::create_dir_all(&path).await?;
            }
            Err(e) => {
                Err(e)?;
            }
        }

        let interval = config
            .dump_interval
            .clone()
            .context("Dump interval is required")?;

        let format = config
            .dump_format
            .clone()
            .context("Dump format is required")?;

        let format_map = HashMap::from([
            ("service".into(), service.name.clone()),
            ("timestamp".into(), "0".into()),
            ("client_ip".into(), service.client_addr.ip().to_string()),
            ("client_port".into(), service.client_addr.port().to_string()),
            ("server_ip".into(), service.server_addr.ip().to_string()),
            ("server_port".into(), service.server_addr.port().to_string()),
        ]);

        let (tx, rx) = mpsc::channel(100);
        let dumper = Dumper {
            path,
            format,
            interval,
            rx,
        };

        tokio::spawn(async move { dumper.dump_pcap(format_map).await });
        Ok(tx)
    }

    async fn dump_pcap(mut self, mut format_map: HashMap<String, String>) -> anyhow::Result<()> {
        loop {
            let deadline = tokio::time::Instant::now() + self.interval;

            let mut tmpfile = NamedTempFile::new()?;
            let mut writer = PcapWriter::new(tmpfile.as_file_mut())?;

            loop {
                tokio::select! {
                    maybe_msg = self.rx.recv() => {
                        if let Some(msg) = maybe_msg {
                            let packet = Self::build_packet(msg.src, msg.dst, msg.time, 0, 0, 0, &msg.chunk)?;
                            writer.write_packet(&packet)?;
                        }
                    }
                    _ = sleep_until(deadline) => {
                        break;
                    }
                }
            }

            let end = Utc::now().timestamp();
            format_map.insert("timestamp".into(), end.to_string());
            let filename = strfmt::strfmt(&self.format, &format_map)?;

            let path = self.path.join(filename);
            match tmpfile.persist(&path) {
                Ok(_) => info!("Dumped pcaps to {}", path.to_string_lossy()),
                Err(e) => error!("Failed to save pcaps to {}: {}", path.to_string_lossy(), e),
            }
        }
    }

    fn build_packet(
        src: SocketAddr,
        dst: SocketAddr,
        timestamp: Duration,
        seq: u32,
        ack: u32,
        flags: u16,
        payload: &[u8],
    ) -> anyhow::Result<PcapPacket<'static>> {
        let (src_ip, src_port, dst_ip, dst_port) = match (src, dst) {
            (SocketAddr::V4(src), SocketAddr::V4(dst)) => {
                (src.ip().clone(), src.port(), dst.ip().clone(), dst.port())
            }
            _ => todo!("ipv6"),
        };

        let mut builder = PacketBuilder::ipv4(src_ip.octets(), dst_ip.octets(), 64)
            .tcp(src_port, dst_port, seq, 1024);

        if flags & 0x10 != 0 {
            builder = builder.ack(ack);
        }
        if flags & 0x01 != 0 {
            builder = builder.fin();
        }
        if flags & 0x02 != 0 {
            builder = builder.syn();
        }
        if flags & 0x08 != 0 {
            builder = builder.psh();
        }

        let mut buf = Vec::with_capacity(builder.size(payload.len()));
        builder.write(&mut buf, payload)?;
        buf.extend_from_slice(payload);

        Ok(PcapPacket {
            timestamp,
            orig_len: buf.len() as u32,
            data: Cow::Owned(buf),
        })
    }

    pub async fn write_chunk(
        channel: &DumperChannel,
        src: SocketAddr,
        dst: SocketAddr,
        chunk: Vec<u8>,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        Ok(channel
            .send(DumperMsg {
                src,
                dst,
                time: Duration::new(now.timestamp() as u64, now.timestamp_subsec_nanos()),
                chunk,
            })
            .await?)
    }
}
