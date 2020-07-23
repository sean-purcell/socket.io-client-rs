use std::error::Error as StdError;

use async_tungstenite::{
    async_tls::{self, ClientStream},
    tungstenite::{Error as WsError, Message as WsMessage},
    WebSocketStream,
};
use futures::{
    channel::mpsc,
    future::{Future, RemoteHandle},
    io::{AsyncRead, AsyncWrite},
    pin_mut, select,
    sink::{Sink, SinkExt},
    stream::{Stream, StreamExt},
    task::{Spawn, SpawnError, SpawnExt},
};
use http::uri::{InvalidUri, Uri};

pub struct Client {
    send: mpsc::UnboundedSender<WsMessage>,
    receive: mpsc::UnboundedReceiver<WsMessage>,
    handle: RemoteHandle<()>,
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

trait AsyncIo: AsyncRead + AsyncWrite + Unpin {}
impl<S> AsyncIo for S where S: AsyncRead + AsyncWrite + Unpin {}

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
        S: 'static + AsyncRead + AsyncWrite + Unpin,
        E: 'static + StdError,
    {
        let uri = uri.as_ref();
        let uri = parse_uri(uri).map_err(|e| Error::UriError(uri.to_string(), e))?;

        let connection = connect(uri.host().unwrap(), uri.port_u16().unwrap())
            .await
            .map_err(|e| Error::ConnectionError(Box::new(e)))?;

        let connection: Box<dyn AsyncIo> = Box::new(connection);

        let (stream, _) = async_tls::client_async_tls(uri, connection).await?;

        let (send, receive, handle) = process_websocket(stream, spawn)?;

        Ok(Client {
            send,
            receive,
            handle,
        })
    }
}

fn process_websocket(
    stream: WebSocketStream<ClientStream<Box<dyn AsyncIo>>>,
    spawn: impl Spawn,
) -> Result<
    (
        mpsc::UnboundedSender<WsMessage>,
        mpsc::UnboundedReceiver<WsMessage>,
        RemoteHandle<()>,
    ),
    SpawnError,
> {
    let (send_tx, send_rx) = mpsc::unbounded();
    let (receive_tx, receive_rx) = mpsc::unbounded();

    let process = || async move {
        pin_mut!(send_rx, stream);
        loop {
            select! {
                msg = send_rx.next() => match stream.send(msg.unwrap() /* FIXME */).await {
                    Ok(()) => (),
                    Err(e) => { return e; }
                },
            }
        }
    };
    let handle = spawn.spawn_with_handle(async { loop {} })?;

    Ok((send_tx, receive_rx, handle))
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
