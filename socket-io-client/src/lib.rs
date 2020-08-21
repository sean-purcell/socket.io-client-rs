#![recursion_limit = "1024"] // Needed for select

use std::{
    error::Error as StdError,
    sync::{Arc, Mutex},
};

use async_tungstenite::{
    async_tls,
    tungstenite::{Error as WsError, Message as WsMessage},
    WebSocketStream,
};
use futures::{
    channel::{mpsc, oneshot},
    future::{Future, FutureExt, RemoteHandle},
    io::{AsyncRead, AsyncWrite},
    select,
    sink::SinkExt,
    stream::StreamExt,
    task::{Spawn, SpawnError, SpawnExt},
};
use url::Url;

use socket_io_protocol::engine;

mod callbacks;
mod emit;
pub mod protocol;
mod receiver;

use callbacks::Callbacks;
pub use callbacks::{AckCallback, EventCallback};
pub use emit::{AckArgsBuilder, AckBuilder, EventArgsBuilder, EventBuilder};
use receiver::Receiver;

type CloseHandle = (oneshot::Sender<()>, RemoteHandle<Result<(), Error>>);

pub struct Client {
    pub send: mpsc::UnboundedSender<Vec<WsMessage>>,
    close_handle: Option<CloseHandle>,
    callbacks: Arc<Mutex<Callbacks>>,
    next_id: u64,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to parse URI {0}: {1}")]
    UrlError(String, UrlError),
    #[error("Websocket error: {0}")]
    WebsocketError(#[from] WsError),
    #[error("Connection error: {0}")]
    ConnectionError(Box<dyn StdError + Send>),
    #[error("Failed to spawn task: {0}")]
    SpawnError(#[from] SpawnError),
    #[error("Error processing packet: {0}")]
    ProcessingError(#[from] receiver::Error),
    #[error("Already closed")]
    AlreadyClosed,
}

#[derive(thiserror::Error, Debug)]
pub enum UrlError {
    #[error(transparent)]
    Parse(#[from] url::ParseError),
    #[error("Invalid scheme: {0:?}")]
    InvalidScheme(String),
    #[error("No host")]
    NoHost,
}

macro_rules! fwd_cbs {
    ($(#[$attrs:meta])* $n1:ident $n2:ident $tgt:ident $inv:expr, ($($arg:ident : $ty:ty),*)) => {
        paste::paste! {
            $(#[$attrs])*
            pub fn $n1(
                &mut self,
                namespace: &str,
                $( $arg : $ty ),*
            ) {
                self.callbacks.lock().unwrap().$tgt(namespace, $( $arg ),*)
            }

            #[doc = "Equivalent to `"]
            #[doc = $inv]
            #[doc = "`."]
            pub fn $n2(&mut self, $( $arg : $ty ),*) {
                self.$n1("/", $( $arg ),*)
            }
        }
    };

    ($(#[$attrs:meta])* $fn1:ident $fn2:ident ($($arg:ident : $ty:ty),*)) => {
            paste::paste! {
        fwd_cbs! {
                $(#[$attrs])*
                [<$fn1 _namespace_ $fn2 _callback>]
                [<$fn1 _ $fn2 _callback>]
                [<$fn1 _ $fn2>]
                stringify!( [<$fn1 _namespace_ $fn2 _callback>] ("/", $($arg),*) ),
                ($($arg : $ty),*)
            }
        }
    };
}

pub type Host = String;
pub type Port = u16;

impl Client {
    pub async fn connect<C, F, S, E>(
        url: impl AsRef<str>,
        connect: C,
        spawn: &impl Spawn,
    ) -> Result<Client, Error>
    where
        C: 'static + Fn(Host, Port) -> F,
        F: Future<Output = Result<S, E>>,
        S: 'static + AsyncRead + AsyncWrite + Unpin + Send,
        E: 'static + StdError + Send,
    {
        let url = url.as_ref();
        let url = parse_url(url).map_err(|e| Error::UrlError(url.to_string(), e))?;

        let connection = connect(
            url.host_str().unwrap().into(),
            url.port_or_known_default().unwrap(),
        )
        .await
        .map_err(|e| Error::ConnectionError(Box::new(e)))?;

        Client::new(url, connection, spawn).await
    }

    pub async fn from_stream<S>(
        url: impl AsRef<str>,
        connection: S,
        spawn: &impl Spawn,
    ) -> Result<Client, Error>
    where
        S: 'static + AsyncRead + AsyncWrite + Unpin + Send,
    {
        let url = url.as_ref();
        let url = parse_url(url).map_err(|e| Error::UrlError(url.to_string(), e))?;

        Client::new(url, connection, spawn).await
    }

    async fn new<S>(mut url: Url, connection: S, spawn: &impl Spawn) -> Result<Client, Error>
    where
        S: 'static + AsyncRead + AsyncWrite + Unpin + Send,
    {
        add_socketio_query_params(&mut url);

        let (stream, _) = async_tls::client_async_tls(url.to_string(), connection).await?;

        let callbacks = Arc::new(Mutex::new(Callbacks::new()));

        let (send, close, open, handle) =
            process_websocket(stream, callbacks.clone(), spawn).await?;

        let open = open.await.unwrap();

        log::info!("Received open: {:?}", open);

        Ok(Client {
            send,
            close_handle: Some((close, handle)),
            callbacks,
            next_id: 0,
        })
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        let (close, handle) = self.close_handle.take().ok_or(Error::AlreadyClosed)?;

        let _ = close.send(());
        handle.await
    }

    /// Create an `EmitBuilder` to emit an event for the given namespace.
    pub fn namespace_emit<'a: 'd, 'b: 'd, 'c: 'd, 'd>(
        &'a mut self,
        namespace: &'b str,
        event: &'c str,
    ) -> EventBuilder<'d> {
        EventBuilder::new(self, event, namespace)
    }

    /// Equivalent to `namespace_emit("/", event)`.
    pub fn emit<'a: 'c, 'b: 'c, 'c>(&'a mut self, event: &'b str) -> EventBuilder<'c> {
        self.namespace_emit("/", event)
    }

    fwd_cbs! {
        /// Set the callback for messages received to this namespace and event.
        set event(event: &str, callback: impl Into<EventCallback>)
    }
    fwd_cbs! {
        /// Clears any callback set for messages received to this namespace and event,
        /// any messages will be routed to the fallback callback if there is one.
        clear event(event: &str)
    }
    fwd_cbs! {
        /// Set the fallback callback for this namespace, which will be called for messages for any
        /// event without a callback set.
        set fallback(callback: impl Into<EventCallback>)
    }
    fwd_cbs! {
        /// Clears the fallback callback for this namespace.
        clear fallback()
    }
}

async fn process_websocket<S>(
    stream: WebSocketStream<S>,
    callbacks: Arc<Mutex<Callbacks>>,
    spawn: &impl Spawn,
) -> Result<
    (
        mpsc::UnboundedSender<Vec<WsMessage>>,
        oneshot::Sender<()>,
        oneshot::Receiver<engine::Open>,
        RemoteHandle<Result<(), Error>>,
    ),
    SpawnError,
>
where
    S: 'static + Unpin + AsyncRead + AsyncWrite + Send,
{
    let (mut sink, mut stream) = stream.split();
    let (send_tx, mut send_rx) = mpsc::unbounded::<Vec<WsMessage>>();
    let (close_tx, close_rx) = oneshot::channel();
    let (open_tx, open_rx) = oneshot::channel();

    let mut receiver = Receiver::new(send_tx.clone(), callbacks, open_tx);

    let task = || async move {
        let mut next = stream.next().fuse();
        let mut closed = close_rx.fuse();
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

    let handle = spawn.spawn_with_handle(task())?;

    Ok((send_tx, close_tx, open_rx, handle))
}

fn parse_url(url: &str) -> Result<Url, UrlError> {
    let mut url = Url::parse(url)?;

    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        s => return Err(UrlError::InvalidScheme(s.to_string())),
    };

    url.set_scheme(scheme).unwrap();
    let _ = url.host().ok_or(UrlError::NoHost)?;

    Ok(url)
}

fn add_socketio_query_params(url: &mut Url) {
    url.query_pairs_mut()
        .append_pair("EIO", "4")
        .append_pair("transport", "websocket");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url() {
        let p = parse_url("https://example.com/").unwrap();
        assert_eq!(p.to_string(), "wss://example.com/");
        assert_eq!(p.port_or_known_default().unwrap(), 443);
        let p = parse_url("http://localhost:8000/").unwrap();
        assert_eq!(p.to_string(), "ws://localhost:8000/");
        let p = parse_url("localhost:8000");
        assert_eq!(format!("{:?}", p), "Err(InvalidScheme(\"localhost\"))");
    }
}
