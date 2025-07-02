pub mod history;
pub mod iter;

use chrono::{DateTime, Utc};
use std::net::SocketAddr;
use uuid::Uuid;

use crate::flow::history::{HttpHistory, RawHistory};

#[enum_dispatch::enum_dispatch(IsFlow)]
pub enum Flow {
    Raw(RawFlow),
    Http(HttpFlow),
}

#[enum_dispatch::enum_dispatch]
pub trait IsFlow {
    fn get_id(&self) -> Uuid;
    fn get_start(&self) -> DateTime<Utc>;
    fn get_client_addr(&self) -> SocketAddr;
    fn get_server_addr(&self) -> SocketAddr;
}

pub struct HttpFlow {
    pub id: Uuid,
    pub start: DateTime<Utc>,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub history: HttpHistory,
}

pub struct RawFlow {
    pub id: Uuid,
    pub start: DateTime<Utc>,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub client_history: RawHistory,
    pub server_history: RawHistory,
}

impl HttpFlow {
    pub fn new(
        client_addr: SocketAddr,
        client_max_history: usize,
        server_addr: SocketAddr,
        server_max_history: usize,
    ) -> HttpFlow {
        HttpFlow {
            id: Uuid::new_v4(),
            start: Utc::now(),
            client_addr,
            server_addr,
            history: HttpHistory::new(client_max_history, server_max_history),
        }
    }
}

impl IsFlow for HttpFlow {
    fn get_id(&self) -> Uuid {
        self.id
    }

    fn get_start(&self) -> DateTime<Utc> {
        self.start
    }

    fn get_client_addr(&self) -> SocketAddr {
        self.client_addr
    }

    fn get_server_addr(&self) -> SocketAddr {
        self.server_addr
    }
}

impl RawFlow {
    pub fn new(
        client_addr: SocketAddr,
        client_max_history: usize,
        server_addr: SocketAddr,
        server_max_history: usize,
    ) -> RawFlow {
        RawFlow {
            id: Uuid::new_v4(),
            start: Utc::now(),
            client_addr,
            server_addr,
            client_history: RawHistory::new(client_max_history),
            server_history: RawHistory::new(server_max_history),
        }
    }
}

impl IsFlow for RawFlow {
    fn get_id(&self) -> Uuid {
        self.id
    }

    fn get_start(&self) -> DateTime<Utc> {
        self.start
    }

    fn get_client_addr(&self) -> SocketAddr {
        self.client_addr
    }

    fn get_server_addr(&self) -> SocketAddr {
        self.server_addr
    }
}
