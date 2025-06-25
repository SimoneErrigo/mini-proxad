use crate::service::ServiceStream;

use async_trait::async_trait;
use std::future::poll_fn;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

#[async_trait]
pub trait ChunkStream: AsyncRead + AsyncWrite + Send + Sync + Unpin {
    async fn read_chunk(&mut self, buffer: &mut Vec<u8>) -> tokio::io::Result<usize>;
    async fn write_chunk(&mut self, buffer: &[u8]) -> tokio::io::Result<()>;
}

#[async_trait]
impl<T> ChunkStream for T
where
    T: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn read_chunk(&mut self, buffer: &mut Vec<u8>) -> tokio::io::Result<usize> {
        poll_fn(|cx| {
            let mut total = 0;
            let mut temp_buf = [0u8; 4096];
            let mut read_buf = ReadBuf::new(&mut temp_buf);

            loop {
                let pinned = Pin::new(&mut *self);
                match pinned.poll_read(cx, &mut read_buf) {
                    Poll::Ready(Ok(())) => {
                        let filled = read_buf.filled();
                        if filled.is_empty() {
                            return Poll::Ready(Ok(total));
                        }

                        buffer.extend_from_slice(filled);
                        total += filled.len();
                    }
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(Err(e));
                    }
                    Poll::Pending => {
                        if total > 0 {
                            return Poll::Ready(Ok(total));
                        } else {
                            return Poll::Pending;
                        }
                    }
                }
            }
        })
        .await
    }

    async fn write_chunk(&mut self, buffer: &[u8]) -> tokio::io::Result<()> {
        self.write_all(buffer).await
    }
}

// TODO: There are problably much better ways to do this
#[async_trait]
pub trait AsyncListener<T: AsyncRead + AsyncWrite + Send + Sync> {
    async fn accept(&self) -> tokio::io::Result<(T, SocketAddr)>;
}

pub struct TlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsListener {
    pub fn new(listener: TcpListener, acceptor: TlsAcceptor) -> Self {
        TlsListener { listener, acceptor }
    }
}

#[async_trait]
impl AsyncListener<ServiceStream> for TlsListener {
    async fn accept(&self) -> tokio::io::Result<(ServiceStream, SocketAddr)> {
        let (stream, addr) = self.listener.accept().await?;
        self.acceptor
            .accept(stream)
            .await
            .map(|stream| (Box::pin(stream) as Pin<Box<dyn ChunkStream>>, addr))
    }
}

#[async_trait]
impl AsyncListener<ServiceStream> for TcpListener {
    async fn accept(&self) -> tokio::io::Result<(ServiceStream, SocketAddr)> {
        self.accept()
            .await
            .map(|(stream, addr)| (Box::pin(stream) as Pin<Box<dyn ChunkStream>>, addr))
    }
}
