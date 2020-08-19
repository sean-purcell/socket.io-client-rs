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

    pub fn binary(&mut self, b: bool) -> &mut Self {
        self.binary = b;
        self
    }

    pub fn callback(&mut self, c: impl Into<AckCallback>) -> &mut Self {
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
    pub fn arg<T>(&mut self, arg: &T) -> Result<&mut Self, ArgsError>
    where
        T: Serialize + ?Sized,
    {
        self.builder.serialize_arg(arg)?;
        Ok(self)
    }

    pub fn finish(self) {
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
