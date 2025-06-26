use std::ops::Range;

use chrono::{DateTime, Utc};
use tokio::time;
use uuid::Uuid;

use crate::proxy::stream::ProxyStream;
use std::time::Duration;

// TODO: Tweak this accordingly
const MAX_HISTORY_SIZE: usize = 1 << 20;

pub enum FlowStatus {
    Read,
    Closed,
    Timeout,
    HistoryTooBig,
}

pub struct Flow {
    pub id: Uuid,
    pub client: ProxyStream,
    pub server_history: History,
    pub server: ProxyStream,
    pub client_history: History,
}

pub struct HistoryChunk {
    pub range: Range<usize>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Default)]
pub struct History {
    pub bytes: Vec<u8>,
    pub chunks: Vec<HistoryChunk>,
}

impl Flow {
    pub fn new(client: ProxyStream, server: ProxyStream) -> Flow {
        Flow {
            id: Uuid::new_v4(),
            client,
            client_history: History::default(),
            server,
            server_history: History::default(),
        }
    }

    pub async fn read_chunk(
        stream: &mut ProxyStream,
        history: &mut History,
        timeout: Duration,
    ) -> anyhow::Result<FlowStatus> {
        let start = history.bytes.len();
        let future = stream.read_chunk(&mut history.bytes);

        match time::timeout(timeout, future).await {
            Ok(Ok(0)) => Ok(FlowStatus::Closed),
            Ok(Ok(n)) => {
                if start + n >= MAX_HISTORY_SIZE {
                    Ok(FlowStatus::HistoryTooBig)
                } else {
                    history.chunks.push(HistoryChunk {
                        range: start..start + n,
                        timestamp: Utc::now(),
                    });
                    Ok(FlowStatus::Read)
                }
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Ok(FlowStatus::Timeout),
        }
    }

    pub async fn write_last_chunk(
        stream: &mut ProxyStream,
        history: &History,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        Ok(time::timeout(timeout, stream.write_chunk(history.last_chunk())).await??)
    }
}

impl History {
    pub fn last_chunk(&self) -> &[u8] {
        let range = self
            .chunks
            .last()
            .map(|chunk| chunk.range.clone())
            .unwrap_or(0..0);
        &self.bytes[range]
    }

    pub fn set_last_chunk(&mut self, bytes: &[u8]) {
        match self.chunks.pop() {
            Some(HistoryChunk { range, timestamp }) => {
                let start = range.start;
                self.bytes.truncate(start);

                self.bytes.extend_from_slice(bytes);
                self.chunks.push(HistoryChunk {
                    range: start..start + bytes.len(),
                    timestamp,
                });
            }
            None => {
                self.bytes.extend_from_slice(bytes);
                self.chunks.push(HistoryChunk {
                    range: 0..bytes.len(),
                    timestamp: Utc::now(),
                });
            }
        }
    }
}
