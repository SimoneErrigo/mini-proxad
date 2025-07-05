use bytes::Bytes;
use chrono::{DateTime, Utc};
use hyper::{Request, Response};
use std::ops::Range;

use crate::http::{HttpRequest, HttpResponse};

pub struct HttpHistory {
    pub requests: Vec<(HttpRequest, DateTime<Utc>)>,
    pub responses: Vec<(HttpResponse, DateTime<Utc>)>,
    pub client_size: usize,
    pub server_size: usize,
    pub client_max: usize,
    pub server_max: usize,
}

impl HttpHistory {
    pub fn new(client_max: usize, server_max: usize) -> Self {
        HttpHistory {
            requests: vec![],
            responses: vec![],
            client_size: 0,
            server_size: 0,
            client_max,
            server_max,
        }
    }

    pub fn push_request(&mut self, req: Request<Bytes>, len: usize) -> bool {
        if len + self.client_size > self.client_max {
            false
        } else {
            self.client_size += len;
            self.requests.push((HttpRequest(req), Utc::now()));
            true
        }
    }

    pub fn push_response(&mut self, resp: Response<Bytes>, len: usize) -> bool {
        if len + self.server_size > self.server_max {
            false
        } else {
            self.server_size += len;
            self.responses.push((HttpResponse(resp), Utc::now()));
            true
        }
    }
}

#[derive(Clone)]
pub struct RawChunk {
    pub range: Range<usize>,
    pub timestamp: DateTime<Utc>,
}

pub struct RawHistory {
    pub bytes: Vec<u8>,
    pub chunks: Vec<RawChunk>,
    pub max_size: usize,
}

impl RawHistory {
    pub fn new(max_size: usize) -> RawHistory {
        RawHistory {
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

    pub fn last_timestamp(&self) -> DateTime<Utc> {
        self.chunks
            .last()
            .map(|chunk| chunk.timestamp.clone())
            .unwrap_or_else(|| Utc::now())
    }

    pub fn set_last_chunk(&mut self, bytes: &[u8]) {
        match self.chunks.pop() {
            Some(RawChunk { range, timestamp }) => {
                let start = range.start;
                self.bytes.truncate(start);

                self.bytes.extend_from_slice(bytes);
                self.chunks.push(RawChunk {
                    range: start..start + bytes.len(),
                    timestamp,
                });
            }
            None => {
                self.bytes.extend_from_slice(bytes);
                self.chunks.push(RawChunk {
                    range: 0..bytes.len(),
                    timestamp: Utc::now(),
                });
            }
        }
    }
}
