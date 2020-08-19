use async_tungstenite::tungstenite::Message as WsMessage;
use futures::channel::mpsc;
use serde::Serialize;

use socket_io_protocol::socket::PacketBuilder;

use super::{protocol::ArgsError, AckCallback, Client};

pub struct EventBuilder<'a> {
    client: &'a mut Client,
    event: &'a str,
    namespace: &'a str,
    binary: bool,
    callback: Option<(AckCallback, u64)>,
}

pub struct EventArgsBuilder<'a> {
    client: &'a mut Client,
    namespace: &'a str,
    callback: Option<(AckCallback, u64)>,
    builder: PacketBuilder,
}

pub struct AckBuilder {
    send: mpsc::UnboundedSender<Vec<WsMessage>>,
    namespace: String,
    id: u64,
    binary: bool,
}

pub struct AckArgsBuilder {
    send: mpsc::UnboundedSender<Vec<WsMessage>>,
    builder: PacketBuilder,
}

impl<'a> EventBuilder<'a> {
    pub(crate) fn new(client: &'a mut Client, event: &'a str, namespace: &'a str) -> Self {
        EventBuilder {
            client,
            event,
            namespace,
            binary: false,
            callback: None,
        }
    }

    pub fn binary(mut self, b: bool) -> Self {
        self.binary = b;
        self
    }

    pub fn callback(mut self, c: impl Into<AckCallback>) -> Self {
        let id = self.client.next_id;
        self.client.next_id += 1;
        self.callback = Some((c.into(), id));
        self
    }

    pub fn args(self) -> EventArgsBuilder<'a> {
        let builder = PacketBuilder::new_event(
            self.event,
            self.namespace,
            self.callback.as_ref().map(|(_, id)| *id),
            self.binary,
        );
        EventArgsBuilder {
            client: self.client,
            namespace: self.namespace,
            callback: self.callback,
            builder,
        }
    }
}

impl<'a> EventArgsBuilder<'a> {
    pub fn arg<T>(mut self, arg: &T) -> Result<Self, ArgsError>
    where
        T: Serialize + ?Sized,
    {
        self.arg_ref(arg)?;
        Ok(self)
    }

    pub fn arg_ref<T>(&mut self, arg: &T) -> Result<(), ArgsError>
    where
        T: Serialize + ?Sized,
    {
        self.builder.serialize_arg(arg)
    }

    pub fn send(self) {
        let packets = self.builder.finish();
        if let Some((callback, id)) = self.callback {
            self.client
                .callbacks
                .lock()
                .unwrap()
                .set_ack(self.namespace, id, callback);
        }
        let _ = self.client.send.unbounded_send(packets); // TODO: Determine if we care about the result.
    }
}

impl AckBuilder {
    pub(crate) fn new(
        send: mpsc::UnboundedSender<Vec<WsMessage>>,
        namespace: impl Into<String>,
        id: u64,
    ) -> Self {
        AckBuilder {
            send,
            namespace: namespace.into(),
            id,
            binary: false,
        }
    }

    pub fn binary(mut self, b: bool) -> Self {
        self.binary = b;
        self
    }

    pub fn args(self) -> AckArgsBuilder {
        let builder = PacketBuilder::new_ack(self.namespace, self.id, self.binary);
        AckArgsBuilder {
            send: self.send,
            builder,
        }
    }
}

impl AckArgsBuilder {
    pub fn arg<T>(mut self, arg: &T) -> Result<Self, ArgsError>
    where
        T: Serialize + ?Sized,
    {
        self.arg_ref(arg)?;
        Ok(self)
    }

    pub fn arg_ref<T>(&mut self, arg: &T) -> Result<(), ArgsError>
    where
        T: Serialize + ?Sized,
    {
        self.builder.serialize_arg(arg)
    }

    pub fn send(self) {
        let packets = self.builder.finish();
        let _ = self.send.unbounded_send(packets); // TODO: Determine if we care about the result.
    }
}
