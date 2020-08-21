use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_tungstenite::{async_tls, tungstenite::Message as WsMessage, WebSocketStream};
use futures::{
    channel::{mpsc, oneshot},
    future::{FutureExt, RemoteHandle},
    io::{AsyncRead, AsyncWrite},
    pin_mut, select,
    sink::SinkExt,
    stream::StreamExt,
    task::{Spawn, SpawnError, SpawnExt},
};
use futures_timer::Delay;
use url::Url;

use socket_io_protocol::engine;

use super::{Callbacks, Error, Receiver};

pub struct Connection {
    handle: Option<RemoteHandle<Result<(), Error>>>,
    close: Option<oneshot::Sender<()>>,
    sid: String,
    send: mpsc::UnboundedSender<Vec<WsMessage>>,
}

impl Connection {
    pub async fn new<S>(
        mut url: Url,
        connection: S,
        sid: Option<&str>,
        callbacks: Arc<Mutex<Callbacks>>,
        timeout: Duration,
        spawn: &impl Spawn,
    ) -> Result<Connection, Error>
    where
        S: 'static + AsyncRead + AsyncWrite + Unpin + Send,
    {
        if let Some(sid) = sid {
            url.query_pairs_mut().append_pair("sid", sid);
        }
        let timeout = Delay::new(timeout).fuse();

        let client = async_tls::client_async_tls(url.to_string(), connection).fuse();
        pin_mut!(client);
        pin_mut!(timeout);

        let client = select! {
            c = client => c.map(|(c, _)| c).map_err(|e| e.into()),
            _ = timeout => Err(Error::Timeout),
        }?;

        let (send_tx, send_rx) = mpsc::unbounded();
        let (close_tx, close_rx) = oneshot::channel();
        let (open_tx, open_rx) = oneshot::channel();

        let handle = process_websocket(
            client,
            send_tx.clone(),
            send_rx,
            close_rx,
            open_tx,
            callbacks,
            spawn,
        )
        .await?;

        let open = open_rx.await.unwrap();
        log::trace!("Received open: {:?}", open);

        Ok(Connection {
            handle: Some(handle),
            close: Some(close_tx),
            sid: open.sid,
            send: send_tx,
        })
    }

    pub fn sid(&self) -> &str {
        &*self.sid
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<Vec<WsMessage>> {
        self.send.clone()
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        if let (Some(handle), Some(close)) = (self.handle.take(), self.close.take()) {
            let _ = close.send(());
            handle.await
        } else {
            Err(Error::AlreadyClosed)
        }
    }
}

async fn process_websocket<S>(
    stream: WebSocketStream<S>,
    send_tx: mpsc::UnboundedSender<Vec<WsMessage>>,
    mut send_rx: mpsc::UnboundedReceiver<Vec<WsMessage>>,
    close: oneshot::Receiver<()>,
    open: oneshot::Sender<engine::Open>,
    callbacks: Arc<Mutex<Callbacks>>,
    spawn: &impl Spawn,
) -> Result<RemoteHandle<Result<(), Error>>, SpawnError>
where
    S: 'static + Unpin + AsyncRead + AsyncWrite + Send,
{
    let (mut sink, mut stream) = stream.split();
    let mut receiver = Receiver::new(send_tx.clone(), callbacks, open);

    let task = async move {
        let mut next = stream.next().fuse();
        let mut closed = close.fuse();
        loop {
            select! {
                result = next => {
                    let msg = match result {
                        Some(msg) => {
                            log::trace!("received message: {:?}", msg);
                            msg
                        },
                        None => {
                            log::trace!("got None, stream ended");
                            return Ok(()); // Connection closed without errors
                        }
                    };
                    next = stream.next().fuse();
                    match msg {
                        Ok(msg) => receiver.process_websocket_packet(msg)?,
                        Err(e) => return Err(e.into()),
                    }
                }
                result = send_rx.next() => {
                    let msgs = match result {
                        Some(msg) => msg,
                        None => panic!("Sending stream closed unexpectedly"),
                    };
                    for msg in msgs.into_iter() {
                        log::trace!("Sending websocket packet: {:?}", msg);
                        match sink.send(msg).await {
                            Ok(()) => (),
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                _ = closed => {
                    break;
                }
            }
        }
        drop(next);
        let mut ws_stream = sink.reunite(stream).expect("Reunite should succeed");
        log::debug!("Sending close message");
        let _ = ws_stream.close(None).await;
        // Now we want to keep reading until the stream closed
        loop {
            match ws_stream.next().await {
                Some(Ok(msg)) => receiver.process_websocket_packet(msg)?,
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()), // Connection closed without errors
            }
        }
    };

    spawn.spawn_with_handle(task)
}
