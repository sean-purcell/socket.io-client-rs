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

mod callbacks;
mod emit;
pub mod protocol;
mod receiver;

use callbacks::Callbacks;
pub use callbacks::{AckCallback, EventCallback};
pub use emit::{EventArgsBuilder, EventBuilder};
use receiver::Receiver;

pub struct Client {
    pub send: mpsc::UnboundedSender<Vec<WsMessage>>,
    close_handle: Option<(oneshot::Sender<()>, RemoteHandle<Result<(), Error>>)>,
    callbacks: Arc<Mutex<Callbacks>>,
    _receiver: Receiver,
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

        let (send, receive, close, handle) = process_websocket(stream, spawn).await?;

        let callbacks = Arc::new(Mutex::new(Callbacks::new()));

        let _receiver = Receiver::new(receive, send.clone(), callbacks.clone(), spawn)?;

        Ok(Client {
            send,
            close_handle: Some((close, handle)),
            callbacks,
            _receiver,
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
    spawn: &impl Spawn,
) -> Result<
    (
        mpsc::UnboundedSender<Vec<WsMessage>>,
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
    let (send_tx, mut send_rx) = mpsc::unbounded::<Vec<WsMessage>>();
    let (mut receive_tx, receive_rx) = mpsc::unbounded();
    let (close_tx, close_rx) = oneshot::channel();

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
                        Ok(msg) => receive_tx
                            .send(msg)
                            .await
                            .expect("Receiver dropped unexpectedly"),
                        Err(e) => return Err(e.into()),
                    }
                }
                result = send_rx.next() => {
                    let msgs = match result {
                        Some(msg) => msg,
                        None => panic!("Sending stream closed unexpectedly"),
                    };
                    for msg in msgs.into_iter() {
                        log::trace!("sending message: {:?}", msg);
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
                Some(Ok(msg)) => receive_tx
                    .send(msg)
                    .await
                    .expect("Receiver dropped unexpectedly"),
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()), // Connection closed without errors
            }
        }
    };

    let handle = spawn.spawn_with_handle(task())?;

    Ok((send_tx, receive_rx, close_tx, handle))
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
    use std::{convert::TryFrom, pin::Pin, sync::Once};

    use futures::{
        io::{self, ErrorKind},
        ready,
        stream::Stream,
        task::{Context, Poll},
    };
    use pin_project::pin_project;

    use super::*;

    static LOG: Once = Once::new();

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
                    ErrorKind::ConnectionReset,
                    "stream closed for read",
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
                            ErrorKind::ConnectionReset,
                            "stream closed for write",
                        )));
                    }
                }
            }
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), io::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), io::Error>> {
            if !self.i.is_closed() {
                self.i.close_channel();
            }
            Poll::Ready(Ok(()))
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

    fn init_log() {
        LOG.call_once(|| env_logger::init());
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
        init_log();

        let spawn = async_executors::TokioTp::try_from(&mut tokio::runtime::Builder::new())
            .expect("build threadpool");

        let spawn_ref = &spawn;

        let test = async move {
            let (s0, s1) = create_connection();
            let server = async_tungstenite::accept_async(s0);
            let client = async_tungstenite::client_async("ws://localhost/", s1);
            let (server, (client, _)) =
                futures::try_join!(server, client).expect("Failed to connect websockets");

            let (_s_send, _s_recv, s_close, s_handle) =
                process_websocket(server, spawn_ref).await.unwrap();
            let (c_send, mut c_recv, c_close, c_handle) =
                process_websocket(client, spawn_ref).await.unwrap();

            c_send
                .unbounded_send(vec![WsMessage::Ping(vec![0xde, 0xad, 0xbe, 0xef])])
                .unwrap();
            let resp = c_recv.next().await.unwrap();
            assert_eq!(resp, WsMessage::Pong(vec![0xde, 0xad, 0xbe, 0xef]));

            let _ = c_close.send(());
            let c_result = c_handle.await;
            assert!(c_result.is_ok());
            let _ = s_close.send(());
            assert!(s_handle.await.is_ok());
        };

        spawn.block_on(test);
    }

    /*
    #[test]
    fn test_close() {
        init_log();

        let spawn = async_executors::TokioTp::try_from(&mut tokio::runtime::Builder::new())
            .expect("build threadpool");

        let spawn_ref = &spawn;

        let test = async move {
            let (s0, s1) = create_connection();
            let server =
                async_tungstenite::accept_async(s0).map(|x| x.map_err(Error::WebsocketError));
            let client = Client::from_stream("ws://localhost/", s1, spawn_ref);
            let (server, mut client) =
                futures::try_join!(server, client).expect("Failed to connect websockets");

            let (_s_send, _s_recv, s_close, s_handle) =
                process_websocket(server, spawn_ref).await.unwrap();

            client
                .send
                .unbounded_send(WsMessage::Ping(vec![0xde, 0xad, 0xbe, 0xef]))
                .unwrap();
            let resp = client.receive.next().await.unwrap();
            assert_eq!(resp, WsMessage::Pong(vec![0xde, 0xad, 0xbe, 0xef]));

            let _ = s_close.send(());
            s_handle.await.unwrap();

            assert_eq!(client.receive.next().await, Some(WsMessage::Close(None)));
            assert_eq!(client.receive.next().await, None);
            client.close().await.unwrap();
        };

        spawn.block_on(test);
    }
    */

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
