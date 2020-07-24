use std::error::Error as StdError;

use async_tungstenite::{
    async_tls::{self, ClientStream},
    tungstenite::{Error as WsError, Message as WsMessage},
    WebSocketStream,
};
use futures::{
    channel::{mpsc, oneshot},
    future::{Future, FutureExt, RemoteHandle},
    io::{AsyncRead, AsyncWrite},
    lock::BiLock,
    pin_mut, select,
    sink::{Sink, SinkExt},
    stream::{SplitSink, SplitStream, Stream, StreamExt, TryStreamExt},
    task::{Spawn, SpawnError, SpawnExt},
};
use http::uri::{InvalidUri, Uri};

pub struct Client {
    send: mpsc::UnboundedSender<WsMessage>,
    receive: mpsc::UnboundedReceiver<WsMessage>,
    close: oneshot::Sender<()>,
    handle: RemoteHandle<Result<(), WsError>>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to parse URI {0}: {1}")]
    UriError(String, UriError),
    #[error("Websocket error: {0}")]
    WebsocketError(#[from] WsError),
    #[error("Connection error: {0}")]
    ConnectionError(Box<dyn StdError>),
    #[error("Failed to spawn task: {0}")]
    SpawnError(#[from] SpawnError),
}

#[derive(thiserror::Error, Debug)]
pub enum UriError {
    #[error(transparent)]
    Parse(#[from] InvalidUri),
    #[error("Invalid scheme: {0:?}")]
    InvalidScheme(Option<String>),
    #[error("No host")]
    NoHost,
}

pub type Host<'a> = &'a str;
pub type Port = u16;

impl Client {
    pub async fn connect<C, F, S, E>(
        uri: impl AsRef<str>,
        connect: C,
        spawn: impl Spawn,
    ) -> Result<Client, Error>
    where
        C: Fn(Host, Port) -> F,
        F: Future<Output = Result<S, E>>,
        S: 'static + AsyncRead + AsyncWrite + Unpin + Send,
        E: 'static + StdError,
    {
        let uri = uri.as_ref();
        let uri = parse_uri(uri).map_err(|e| Error::UriError(uri.to_string(), e))?;

        let connection = connect(uri.host().unwrap(), uri.port_u16().unwrap())
            .await
            .map_err(|e| Error::ConnectionError(Box::new(e)))?;

        let (stream, _) = async_tls::client_async_tls(uri, connection).await?;

        let (send, receive, close, handle) = process_websocket(stream, &spawn).await?;

        Ok(Client {
            send,
            receive,
            close,
            handle,
        })
    }
}

async fn process_websocket<S>(
    stream: WebSocketStream<S>,
    spawn: &impl Spawn,
) -> Result<
    (
        mpsc::UnboundedSender<WsMessage>,
        mpsc::UnboundedReceiver<WsMessage>,
        oneshot::Sender<()>,
        RemoteHandle<Result<(), Error>>,
    ),
    SpawnError,
>
where
    S: 'static + Unpin + AsyncRead + AsyncWrite + Send,
{
    let (sink, stream) = stream.split();
    let (sink_0, sink_1) = BiLock::new(sink);
    let (stream_0, stream_1) = BiLock::new(stream);

    let (send, send_handle) = process_sink(sink_0, spawn).await?;
    let (receive, receive_handle) = process_stream(stream_0, send.clone(), spawn).await?;

    let (close_tx, close_rx) = oneshot::channel();

    let task = || async move {
        select! {
            res = send_handle => {
                Ok(res?)
            }
            res = receive_handle => {
                Ok(res?)
            }
            _ = close_rx => {
                drop(send_handle);
                drop(receive_handle);
                let sink = sink_1.lock().await
            }
        }
    };

    Ok((send, receive, close_tx, receive_handle))
}

async fn process_sink<S>(
    sink: BiLock<S>,
    spawn: &impl Spawn,
) -> Result<
    (
        mpsc::UnboundedSender<WsMessage>,
        RemoteHandle<Result<(), WsError>>,
    ),
    SpawnError,
>
where
    S: 'static + Sink<WsMessage, Error = WsError> + Send,
{
    let (started_tx, started_rx) = oneshot::channel();
    let (send_tx, send_rx) = mpsc::unbounded();

    let task = || async move {
        let mut sink = sink.lock().await;
        let sink = sink.as_pin_mut();
        started_tx.send(());

        send_rx.map(|msg| Ok(msg)).forward(sink).await
    };

    let handle = spawn.spawn_with_handle(task())?;
    started_rx.await;

    Ok((send_tx, handle))
}

async fn process_stream<S>(
    stream: BiLock<S>,
    mut send: mpsc::UnboundedSender<WsMessage>,
    spawn: &impl Spawn,
) -> Result<
    (
        mpsc::UnboundedReceiver<WsMessage>,
        RemoteHandle<Result<(), WsError>>,
    ),
    SpawnError,
>
where
    S: 'static + Stream<Item = Result<WsMessage, WsError>> + Send,
{
    let (started_tx, started_rx) = oneshot::channel();
    let (mut receive_tx, receive_rx) = mpsc::unbounded();

    let task = || async move {
        let mut stream = stream.lock().await;
        started_tx.send(());

        let mut stream = stream.as_pin_mut();
        loop {
            let result = match stream.next().await {
                Some(result) => result,
                None => panic!("Stream shouldn't end without an Error"),
            };
            match result {
                Ok(msg) => {
                    match &msg {
                        WsMessage::Ping(data) => {
                            send.send(WsMessage::Pong(data.clone())).await;
                        }
                        _ => (),
                    }
                    let _ = receive_tx.send(msg).await;
                }
                Err(e) => return Err(e),
            }
        }
    };

    let handle = spawn.spawn_with_handle(task())?;
    let _ = started_rx.await;

    Ok((receive_rx, handle))
}

fn parse_uri(uri: &str) -> Result<Uri, UriError> {
    use std::convert::TryFrom;

    let uri = Uri::try_from(uri)?;

    let (scheme, port) = match uri.scheme_str() {
        Some("http") | Some("ws") => ("ws", 80),
        Some("https") | Some("wss") => ("wss", 443),
        s => return Err(UriError::InvalidScheme(s.map(|s| s.to_string()))),
    };

    let host = uri.host().ok_or(UriError::NoHost)?;
    let port = uri.port_u16().unwrap_or(port);
    let path_and_query = uri.path_and_query().map(|x| x.as_str()).unwrap_or("/");
    Ok(Uri::try_from(format!("{}://{}:{}{}", scheme, host, port, path_and_query)).unwrap())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_uri() {
        let p = parse_uri("https://example.com/").unwrap();
        assert_eq!(p, "wss://example.com:443/");
    }
}
