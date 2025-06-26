use anyhow::Context;
use anyhow::anyhow;
use chrono::Utc;
use etherparse::PacketBuilder;
use pcap_file::pcap::{PcapPacket, PcapWriter};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::sync::mpsc;
use std::{fs::File, path::PathBuf, time::Duration};
use tempfile::NamedTempFile;
use tokio::fs;
use tokio::time::Instant;
use tracing::{error, info, warn};

use crate::proxy::flow::Flow;
use crate::{config::Config, service::Service};

// Dump ignoring the interval
const MAX_PACKETS: usize = 100;

pub struct Dumper {
    path: PathBuf,
    format: String,
    interval: Duration,
    rx: mpsc::Receiver<Flow>,
}

pub type DumperChannel = mpsc::Sender<Flow>;

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

        let (tx, rx) = mpsc::channel();
        let dumper = Dumper {
            path,
            format,
            interval,
            rx,
        };

        tokio::task::spawn_blocking(move || dumper.dumper(format_map));
        Ok(tx)
    }

    fn dumper(self, mut format_map: HashMap<String, String>) -> anyhow::Result<()> {
        loop {
            let mut tmpfile = NamedTempFile::new()?;
            let mut writer = PcapWriter::new(tmpfile.as_file_mut())?;

            let mut n_packets = 0;
            let start = Instant::now();
            loop {
                let elapsed = start.elapsed();
                if elapsed >= self.interval || n_packets > MAX_PACKETS {
                    break;
                }
                let timeout = self.interval - elapsed;

                match self.rx.recv_timeout(timeout) {
                    Ok(flow) => match Self::write_tcp_flow(&mut writer, &flow) {
                        Ok(n) => n_packets += n,
                        Err(e) => warn!("Failed to dump pcaps for flow {}: {}", flow.id, e),
                    },
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        warn!("Dumper channel closed");
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

    pub fn write_tcp_flow(
        writer: &mut PcapWriter<&mut File>,
        flow: &Flow,
    ) -> anyhow::Result<usize> {
        let src_ip = match flow.client_addr.ip() {
            std::net::IpAddr::V4(ip) => ip,
            _ => anyhow::bail!("Only IPv4 supported"),
        };

        let dst_ip = match flow.server_addr.ip() {
            std::net::IpAddr::V4(ip) => ip,
            _ => anyhow::bail!("Only IPv4 supported"),
        };

        let src_port = flow.client_addr.port();
        let dst_port = flow.server_addr.port();

        let mut seq_client = 1_000;
        let mut seq_server = 1_000_000;
        let mut ack_client = seq_server + 1;
        let mut ack_server = seq_client + 1;

        let mut n_packets = 0;

        let mut timestamp = flow
            .client_history
            .chunks
            .first()
            .map(|chunk| chunk.timestamp)
            .map(|timestamp| {
                Duration::new(
                    timestamp.timestamp() as u64,
                    timestamp.timestamp_subsec_nanos(),
                )
            })
            .ok_or_else(|| anyhow!("Malformed flow with no chunks"))?;

        Self::write_tcp_handshake(
            writer,
            timestamp,
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            &mut seq_client,
            &mut ack_client,
            &mut seq_server,
        )?;
        n_packets += 3;

        let mut last_was_client = false;

        for (addr, chunk) in flow.into_iter() {
            let (bytes, seq, ack, src, dst, sport, dport) = if addr == flow.client_addr {
                let bytes = &flow.client_history.bytes[chunk.range.clone()];
                last_was_client = true;

                (
                    bytes, seq_client, ack_client, src_ip, dst_ip, src_port, dst_port,
                )
            } else {
                let bytes = &flow.server_history.bytes[chunk.range.clone()];
                last_was_client = false;

                (
                    bytes, seq_server, ack_server, dst_ip, src_ip, dst_port, src_port,
                )
            };

            timestamp = Duration::new(
                chunk.timestamp.timestamp() as u64,
                chunk.timestamp.timestamp_subsec_nanos(),
            );

            // PSH+ACK data packet
            Self::write_tcp_packet(
                writer, timestamp, src, dst, sport, dport, seq, ack, 0x18, bytes,
            )?;
            n_packets += 1;

            // Update seq/ack numbers per side
            if addr == flow.client_addr {
                seq_client = seq_client.wrapping_add(bytes.len() as u32);
                ack_server = seq_client;
            } else {
                seq_server = seq_server.wrapping_add(bytes.len() as u32);
                ack_client = seq_server;
            }
        }

        if last_was_client {
            // Client sends FIN+ACK
            Self::write_tcp_packet(
                writer,
                timestamp,
                src_ip,
                dst_ip,
                src_port,
                dst_port,
                seq_client,
                ack_client,
                0x11,
                &[],
            )?;
        } else {
            // Server sends FIN
            Self::write_tcp_packet(
                writer,
                timestamp,
                dst_ip,
                src_ip,
                dst_port,
                src_port,
                seq_server,
                ack_server,
                0x11,
                &[],
            )?;
        }
        n_packets += 1;

        Ok(n_packets)
    }

    fn write_tcp_handshake(
        writer: &mut PcapWriter<&mut File>,
        timestamp: Duration,
        src_ip: Ipv4Addr,
        dst_ip: Ipv4Addr,
        src_port: u16,
        dst_port: u16,
        seq_client: &mut u32,
        ack_client: &mut u32,
        seq_server: &mut u32,
    ) -> anyhow::Result<()> {
        // SYN from client
        Self::write_tcp_packet(
            writer,
            timestamp,
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            *seq_client,
            *ack_client,
            0x02,
            &[],
        )?;
        *seq_client += 1;

        // SYN-ACK from server
        Self::write_tcp_packet(
            writer,
            timestamp,
            dst_ip,
            src_ip,
            dst_port,
            src_port,
            *seq_server,
            *seq_client,
            0x12,
            &[],
        )?;

        *seq_server = seq_server.wrapping_add(1);
        *ack_client = *seq_server;

        // ACK from client
        Self::write_tcp_packet(
            writer,
            timestamp,
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            *seq_client,
            *ack_client,
            0x10,
            &[],
        )?;

        Ok(())
    }

    fn write_tcp_packet(
        writer: &mut PcapWriter<&mut File>,
        timestamp: Duration,
        src_ip: Ipv4Addr,
        dst_ip: Ipv4Addr,
        src_port: u16,
        dst_port: u16,
        seq: u32,
        ack: u32,
        flags: u8,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        let window_size = 65535;

        // Use dummy MAC addresses for ethernet layer
        let dummy_mac1 = [0x11; 6];
        let dummy_mac2 = [0x22; 6];

        let mut builder = PacketBuilder::ethernet2(dummy_mac1, dummy_mac2)
            .ipv4(src_ip.octets(), dst_ip.octets(), 64)
            .tcp(src_port, dst_port, seq, window_size);

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

        let mut buffer = Vec::<u8>::with_capacity(14 + 20 + 20 + payload.len());
        builder.write(&mut buffer, payload)?;

        let packet = PcapPacket {
            timestamp,
            orig_len: buffer.len() as u32,
            data: Cow::Owned(buffer),
        };

        writer
            .write_packet(&packet)
            .context("Failed to write packet to pcap")?;

        Ok(())
    }
}
