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
    use std::{convert::TryFrom, pin::Pin};

    use futures::{
        io::{self, ErrorKind},
        ready,
        task::{Context, Poll},
    };
    use pin_project::pin_project;

    use super::*;

    #[pin_project(project = ReadProj)]
    struct Read {
        #[pin]
        i: mpsc::UnboundedReceiver<u8>,
    }

    struct Write {
        i: mpsc::UnboundedSender<u8>,
    }

    #[pin_project(project = RwProj)]
    struct ReadWrite {
        #[pin]
        r: Read,
        #[pin]
        w: Write,
    }

    impl AsyncRead for Read {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context,
            buf: &mut [u8],
        ) -> Poll<Result<usize, io::Error>> {
            if buf.len() == 0 {
                return Poll::Ready(Ok(0));
            }
            let ReadProj { i } = self.project();
            let next = ready!(i.poll_next(cx));
            let result = match next {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Err(io::Error::new(
                    ErrorKind::ConnectionAborted,
                    "stream closed",
                )),
            };

            Poll::Ready(result)
        }
    }

    impl AsyncWrite for Write {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context,
            buf: &[u8],
        ) -> Poll<Result<usize, io::Error>> {
            for b in buf.iter() {
                match self.i.unbounded_send(*b) {
                    Ok(()) => (),
                    Err(e) => {
                        assert!(e.is_disconnected());
                        return Poll::Ready(Err(io::Error::new(
                            ErrorKind::ConnectionAborted,
                            "stream closed",
                        )));
                    }
                }
            }
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), io::Error>> {
            Poll::Ready(match self.i.is_closed() {
                false => Ok(()),
                true => Err(io::Error::new(
                    ErrorKind::ConnectionAborted,
                    "stream closed",
                )),
            })
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), io::Error>> {
            Poll::Ready(match self.i.is_closed() {
                false => {
                    self.i.close_channel();
                    Ok(())
                }
                true => Err(io::Error::new(
                    ErrorKind::ConnectionAborted,
                    "stream closed",
                )),
            })
        }
    }

    impl AsyncRead for ReadWrite {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context,
            buf: &mut [u8],
        ) -> Poll<Result<usize, io::Error>> {
            let RwProj { r, .. } = self.project();
            r.poll_read(cx, buf)
        }
    }

    impl AsyncWrite for ReadWrite {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context,
            buf: &[u8],
        ) -> Poll<Result<usize, io::Error>> {
            let RwProj { w, .. } = self.project();
            w.poll_write(cx, buf)
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
            let RwProj { w, .. } = self.project();
            w.poll_flush(cx)
        }

        fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
            let RwProj { w, .. } = self.project();
            w.poll_close(cx)
        }
    }

    fn create_io_streams() -> (Read, Write) {
        let (send, receive) = mpsc::unbounded();
        (Read { i: receive }, Write { i: send })
    }

    fn create_connection() -> (ReadWrite, ReadWrite) {
        let (r0, w0) = create_io_streams();
        let (r1, w1) = create_io_streams();

        (ReadWrite { r: r0, w: w1 }, ReadWrite { r: r1, w: w0 })
    }

    #[test]
    fn test_process_websocket() {
        let spawn = async_executors::TokioTp::try_from(&mut tokio::runtime::Builder::new())
            .expect("build threadpool");

        let spawn_ref = &spawn;

        let test = async move {
            let (s0, s1) = create_connection();
            let server = async_tungstenite::accept_async(s0);
            let client = async_tungstenite::client_async("ws://localhost/", s1);
            let (server, (client, _)) =
                futures::try_join!(server, client).expect("Failed to connect servers");

            let (_s_send, _s_recv, s_close, s_handle) =
                process_websocket(server, spawn_ref).await.unwrap();
            let (mut c_send, mut c_recv, mut c_close, c_handle) =
                process_websocket(client, spawn_ref).await.unwrap();

            c_send.unbounded_send(WsMessage::Ping(vec![0xde, 0xad, 0xbe, 0xef]));
            let resp = c_recv.next().await.unwrap();
            assert_eq!(resp, WsMessage::Pong(vec![0xde, 0xad, 0xbe, 0xef]));

            let _ = c_close.send(());
            assert!(c_handle.await.is_ok());
            let _ = s_close.send(());
            assert_eq!(format!("{:?}", s_handle.await), "Err(WebsocketError(Io(Custom { kind: ConnectionAborted, error: \"stream closed\" })))");
        };

        spawn.block_on(test);
    }

    #[test]
    fn test_parse_uri() {
        let p = parse_uri("https://example.com/").unwrap();
        assert_eq!(p, "wss://example.com:443/");
    }
}
