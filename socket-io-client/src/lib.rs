#![recursion_limit = "1024"] // Needed for select

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
    handle: RemoteHandle<Result<(), Error>>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to parse URI {0}: {1}")]
    UriError(String, UriError),
    #[error("Websocket error: {0}")]
    WebsocketError(#[from] WsError),
    #[error("Connection error: {0}")]
    ConnectionError(Box<dyn StdError + Send>),
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
        E: 'static + StdError + Send,
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
    let (mut sink, mut stream) = stream.split();
    let (send_tx, mut send_rx) = mpsc::unbounded();
    let (mut receive_tx, receive_rx) = mpsc::unbounded();
    let (close_tx, close_rx) = oneshot::channel();

    let task = || async move {
        let mut next = stream.next().fuse();
        let mut closed = close_rx.fuse();
        loop {
            select! {
                result = next => {
                    let msg = match result {
                        Some(msg) => msg,
                        None => panic!("WebSocketStream closed unexpectedly"),
                    };
                    next = stream.next().fuse();
                    match msg {
                        Ok(msg) => receive_tx
                            .send(msg)
                            .await
                            .expect("Receiver dropped unexpectedly"),
                        Err(e) => return Err(e.into()),
                    }
                }
                result = send_rx.next() => {
                    let msg = match result {
                        Some(msg) => msg,
                        None => panic!("Sending stream closed unexpectedly"),
                    };
                    match sink.send(msg).await {
                        Ok(()) => (),
                        Err(e) => return Err(e.into()),
                    }
                }
                _ = closed => {
                    break;
                }
            }
        }
        drop(next);
        let mut ws_stream = sink.reunite(stream).expect("Reunite should succeed");
        let _ = ws_stream.close(None).await;
        Ok(())
    };

    let handle = spawn.spawn_with_handle(task())?;

    Ok((send_tx, receive_rx, close_tx, handle))
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
