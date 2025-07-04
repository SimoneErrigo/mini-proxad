use std::{borrow::Cow, net::SocketAddr};

use chrono::{DateTime, Utc};
use either::Either;

use crate::{
    flow::{Flow, HttpFlow, RawFlow, history::RawChunk},
    http::{HttpRequest, HttpResponse},
};

impl<'a> IntoIterator for &'a Flow {
    type Item = (SocketAddr, DateTime<Utc>, Cow<'a, [u8]>);
    type IntoIter = FlowIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Flow::Raw(raw) => FlowIterator::Raw(raw.into_iter()),
            Flow::Http(http) => FlowIterator::Http(http.into_iter()),
        }
    }
}

pub enum FlowIterator<'a> {
    Raw(RawFlowIterator<'a>),
    Http(HttpFlowIterator<'a>),
}

impl<'a> Iterator for FlowIterator<'a> {
    type Item = (SocketAddr, DateTime<Utc>, Cow<'a, [u8]>);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FlowIterator::Raw(raw) => raw.next().map(|(addr, chunk)| {
                let bytes = if addr == raw.flow.client_addr {
                    Cow::Borrowed(&raw.flow.client_history.bytes[chunk.range.clone()])
                } else {
                    Cow::Borrowed(&raw.flow.server_history.bytes[chunk.range.clone()])
                };
                (addr, chunk.timestamp, bytes)
            }),
            // FIXME: Handle properly the case when to_bytes fails...
            FlowIterator::Http(http) => http.next().map(|(addr, ts, http)| match http {
                Either::Left(req) => (
                    addr,
                    ts,
                    req.to_bytes()
                        .map(|b| Cow::Owned(b))
                        .unwrap_or(Cow::Borrowed(&[] as &[u8])),
                ),
                Either::Right(resp) => (
                    addr,
                    ts,
                    resp.to_bytes()
                        .map(|b| Cow::Owned(b))
                        .unwrap_or(Cow::Borrowed(&[] as &[u8])),
                ),
            }),
        }
    }
}

pub type EitherHttp<'a> = Either<&'a HttpRequest, &'a HttpResponse>;

impl<'a> IntoIterator for &'a HttpFlow {
    type Item = (SocketAddr, DateTime<Utc>, EitherHttp<'a>);
    type IntoIter = HttpFlowIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        HttpFlowIterator {
            flow: self,
            client_index: 0,
            server_index: 0,
        }
    }
}

pub struct HttpFlowIterator<'a> {
    flow: &'a HttpFlow,
    client_index: usize,
    server_index: usize,
}

impl<'a> Iterator for HttpFlowIterator<'a> {
    type Item = (SocketAddr, DateTime<Utc>, EitherHttp<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.client_index <= self.server_index {
            // grab the (HttpRequest, DateTime) tuple by reference
            self.flow
                .history
                .requests
                .get(self.client_index)
                .map(|(req, ts)| {
                    self.client_index += 1;
                    (self.flow.client_addr, *ts, Either::Left(req))
                })
        } else {
            self.flow
                .history
                .responses
                .get(self.server_index)
                .map(|(resp, ts)| {
                    self.server_index += 1;
                    (self.flow.server_addr, *ts, Either::Right(resp))
                })
        }
    }
}

impl<'a> IntoIterator for &'a RawFlow {
    type Item = (SocketAddr, RawChunk);
    type IntoIter = RawFlowIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        RawFlowIterator {
            flow: self,
            client_index: 0,
            server_index: 0,
        }
    }
}

pub struct RawFlowIterator<'a> {
    pub flow: &'a RawFlow,
    client_index: usize,
    server_index: usize,
}

impl<'a> Iterator for RawFlowIterator<'a> {
    type Item = (SocketAddr, RawChunk);

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
