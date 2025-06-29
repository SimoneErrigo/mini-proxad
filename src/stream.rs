use async_trait::async_trait;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

// TODO: Use real streams

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

            loop {
                let mut read_buf = ReadBuf::new(&mut temp_buf);
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
                        // Don't throw away buffer on EOFs (for example TLS termination without close notify)
                        if e.kind() == tokio::io::ErrorKind::UnexpectedEof {
                            return Poll::Ready(Ok(total));
                        }
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
