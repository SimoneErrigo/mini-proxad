use std::{net::SocketAddr, ops::Range};
use chrono::{DateTime, Utc};
use uuid::Uuid;

pub struct Flow {
    pub id: Uuid,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub client_history: History,
    pub server_history: History,
}

#[derive(Clone)]
pub struct HistoryChunk {
    pub range: Range<usize>,
    pub timestamp: DateTime<Utc>,
}

pub struct History {
    pub bytes: Vec<u8>,
    pub chunks: Vec<HistoryChunk>,
    pub max_size: usize,
}

impl Flow {
    pub fn new(
        client_addr: SocketAddr,
        client_max_history: usize,
        server_addr: SocketAddr,
        server_max_history: usize,
    ) -> Flow {
        Flow {
            id: Uuid::new_v4(),
            client_addr,
            server_addr,
            client_history: History::new(client_max_history),
            server_history: History::new(server_max_history),
        }
    }
}

impl History {
    pub fn new(max_size: usize) -> History {
        History {
            bytes: vec![],
            chunks: vec![],
            max_size,
        }
    }

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

impl<'a> IntoIterator for &'a Flow {
    type Item = (SocketAddr, HistoryChunk);
    type IntoIter = FlowIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        FlowIterator {
            flow: self,
            client_index: 0,
            server_index: 0,
        }
    }
}

pub struct FlowIterator<'a> {
    flow: &'a Flow,
    client_index: usize,
    server_index: usize,
}

impl<'a> Iterator for FlowIterator<'a> {
    type Item = (SocketAddr, HistoryChunk);
    fn next(&mut self) -> Option<Self::Item> {
        let client_chunk = self.flow.client_history.chunks.get(self.client_index);
        let server_chunk = self.flow.server_history.chunks.get(self.server_index);

        match (client_chunk, server_chunk) {
            (Some(client), Some(server)) => {
                // Take the first in chronological order
                if client.timestamp < server.timestamp {
                    self.client_index += 1;
                    Some((self.flow.client_addr, client.clone()))
                } else {
                    self.server_index += 1;
                    Some((self.flow.server_addr, server.clone()))
                }
            }
            (Some(client), None) => {
                self.client_index += 1;
                Some((self.flow.client_addr, client.clone()))
            }
            (None, Some(server)) => {
                self.server_index += 1;
                Some((self.flow.server_addr, server.clone()))
            }
            (None, None) => None,
        }
    }
}
