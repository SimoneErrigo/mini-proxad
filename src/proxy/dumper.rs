use anyhow::Context;
use anyhow::anyhow;
use chrono::Utc;
use etherparse::PacketBuilder;
use pcap_file::pcap::{PcapPacket, PcapWriter};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Cursor;
use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::sync::mpsc;
use std::{path::PathBuf, time::Duration};
use tempfile::NamedTempFile;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use crate::flow::Flow;
use crate::{config::Config, service::Service};

const DUMP_CHANNEL_LIMIT: usize = 400;

const MTU_LEN: usize = 65535;
const ETHERNET_HEADER_LEN: usize = 14;
const IPV4_HEADER_LEN: usize = 20;
const TCP_HEADER_LEN: usize = 20;

pub struct Dumper {
    path: PathBuf,
    format: String,
    interval: Duration,
    max_packets: usize,
    format_map: HashMap<String, String>,
    rx: mpsc::Receiver<Flow>,
}

pub type DumperChannel = mpsc::SyncSender<Flow>;

impl Dumper {
    pub async fn start(service: &Service, config: &Config) -> anyhow::Result<DumperChannel> {
        let path = PathBuf::from(config.dump_path.clone().context("Dump path is required")?);

        match tokio::fs::metadata(&path).await {
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
            ("client_ip".into(), service.client_addr.ip().to_string()),
            ("client_port".into(), service.client_addr.port().to_string()),
            ("server_ip".into(), service.server_addr.ip().to_string()),
            ("server_port".into(), service.server_addr.port().to_string()),
            ("from_ip".into(), service.client_addr.ip().to_string()),
            ("from_port".into(), service.client_addr.port().to_string()),
            ("to_ip".into(), service.server_addr.ip().to_string()),
            ("to_port".into(), service.server_addr.port().to_string()),
        ]);

        let (tx, rx) = mpsc::sync_channel(DUMP_CHANNEL_LIMIT);
        let dumper = Dumper {
            path,
            format,
            interval,
            format_map,
            max_packets: config.dump_max_packets,
            rx,
        };

        tokio::task::spawn_blocking(move || dumper.dumper());
        Ok(tx)
    }

    fn dumper(mut self) -> anyhow::Result<()> {
        loop {
            let mut tmpfile = NamedTempFile::new()?;
            let mut writer = PcapWriter::new(tmpfile.as_file_mut())?;

            let mut n_packets = 0;
            let start = Instant::now();
            loop {
                let elapsed = start.elapsed();
                if elapsed >= self.interval || n_packets > self.max_packets {
                    break;
                }
                let timeout = self.interval - elapsed;

                match self.rx.recv_timeout(timeout) {
                    Ok(flow) => match Self::write_tcp_flow(&mut writer, &flow) {
                        Ok(n) => n_packets += n,
                        Err(e) => warn!("Failed to dump pcaps for flow {}: {:?}", flow.id, e),
                    },
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        warn!("Dumper channel closed");
                        break;
                    }
                }
            }

            if n_packets == 0 {
                info!("Skipped dumping empty pcap file");
                continue;
            }

            let end = Utc::now().timestamp();
            self.format_map.insert("timestamp".into(), end.to_string());
            let filename = strfmt::strfmt(&self.format, &self.format_map)?;

            let path = self.path.join(filename);
            match Self::save_pcap(tmpfile, &path) {
                Ok(_) => info!("Dumped pcaps to {}", path.to_string_lossy()),
                Err(e) => error!(
                    "Failed to save pcaps to {}: {:?}",
                    path.to_string_lossy(),
                    e
                ),
            }
        }
    }

    fn save_pcap(tmpfile: NamedTempFile, path: &std::path::Path) -> anyhow::Result<()> {
        match tmpfile.persist(&path) {
            Ok(_) => (),
            Err(e) => {
                // Non atomic copy if the error is EXDEV
                if e.error.raw_os_error() == Some(18) {
                    let temp_path = e.file.path();
                    fs::copy(&temp_path, path)?;
                    fs::remove_file(&temp_path)?;
                } else {
                    Err(e.error)?
                }
            }
        }
        Ok(())
    }

    fn write_tcp_flow(writer: &mut PcapWriter<&mut File>, flow: &Flow) -> anyhow::Result<usize> {
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

        let header_size = ETHERNET_HEADER_LEN + IPV4_HEADER_LEN + TCP_HEADER_LEN;
        let max_payload = MTU_LEN - header_size;

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

        // TODO: Coalesce consecutive chunks coming from the same source
        for (addr, chunk) in flow.into_iter() {
            timestamp = Duration::new(
                chunk.timestamp.timestamp() as u64,
                chunk.timestamp.timestamp_subsec_nanos(),
            );

            let bytes = if addr == flow.client_addr {
                last_was_client = true;
                &flow.client_history.bytes[chunk.range.clone()]
            } else {
                last_was_client = false;
                &flow.server_history.bytes[chunk.range.clone()]
            };

            let mut offset = 0;
            while offset < bytes.len() {
                let end = (offset + max_payload).min(bytes.len());
                let bytes = &bytes[offset..end];
                offset = end;

                let (seq, ack, src, dst, sport, dport) = if addr == flow.client_addr {
                    (seq_client, ack_client, src_ip, dst_ip, src_port, dst_port)
                } else {
                    (seq_server, ack_server, dst_ip, src_ip, dst_port, src_port)
                };

                let mut flags = 0x10; // ACK
                if end == bytes.len() {
                    flags |= 0x08; // PSH
                }

                Self::write_tcp_packet(
                    writer, timestamp, src, dst, sport, dport, seq, ack, flags, bytes,
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
        *seq_client = seq_client.wrapping_add(1);

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
        // Use dummy MAC addresses for ethernet layer
        let dummy_mac1 = [0x11; 6];
        let dummy_mac2 = [0x22; 6];

        let window_size = 65535;
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

        let mut buffer = [0u8; MTU_LEN];
        let mut cursor = Cursor::new(&mut buffer[..]);
        builder.write(&mut cursor, payload)?;

        let packet_len = cursor.position() as usize;
        let packet = PcapPacket {
            timestamp,
            orig_len: packet_len as u32,
            data: Cow::Borrowed(&buffer[..packet_len]),
        };

        writer
            .write_packet(&packet)
            .context("Failed to write packet to pcap")?;

        Ok(())
    }
}
