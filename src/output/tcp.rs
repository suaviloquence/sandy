use std::{io, net::SocketAddr, sync::Arc};

use tokio::{
    io::AsyncWriteExt,
    net::{tcp::OwnedWriteHalf, TcpListener},
};

use crate::runner::Current;

use super::Message;

#[derive(Debug)]
pub struct Tcp {
    current: Arc<Current>,
}

impl Tcp {
    pub fn new(current: Arc<Current>) -> Self {
        Self { current }
    }

    pub async fn run_loop(self) -> io::Result<()> {
        let server = {
            let current = self.current;

            async move {
                let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 3615))).await?;

                loop {
                    match listener.accept().await {
                        Ok((stream, _addr)) => {
                            let (_reader, writer) = stream.into_split();
                            let current = Arc::clone(&current);
                            let rx = current.tail.read().await.clone();

                            tokio::spawn(async move {
                                if let Err(e) = writer_loop(rx, writer, current).await {
                                    log::info!("Write to stream error: {:?}", e);
                                }
                            });

                            // TODO: reader
                        }
                        Err(e) => log::error!("Error accepting connection: {:?}", e),
                    }
                }
            }
        };

        server.await
    }
}

async fn writer_loop(
    mut rx: lighthouse::Receiver<Message>,
    mut writer: OwnedWriteHalf,
    current: Arc<Current>,
) -> io::Result<()> {
    let guard = current.chunk.read().await;
    if let Some(frames) = guard.as_ref() {
        for frame in frames {
            frame.write(&mut writer).await?;
        }
        writer.flush().await?;
    }

    drop(guard);

    while let Ok(msg) = rx.recv().await {
        match msg.as_ref() {
            Message::Next(_) => {
                // writer.write(&id3).await?;
                // jk dont do anything - no metadata allowed in the middle of a stream
            }
            Message::Frames(frames) => {
                for frame in frames.iter() {
                    frame.write(&mut writer).await?;
                }

                writer.flush().await?;
            }
        }
    }

    Ok(())
}
