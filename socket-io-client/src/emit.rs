use super::{AckCallback, Client};
use socket_io_protocol::socket::PacketBuilder;

pub struct EventBuilder<'a> {
    client: &'a mut Client,
    event: &'a str,
    namespace: Option<&'a str>,
    binary: bool,
    callback: Option<(AckCallback, u64)>,
}

pub struct EventArgsBuilder<'a> {
    client: &'a mut Client,
    callback: Option<(AckCallback, u64)>,
    builder: PacketBuilder,
}

impl<'a> EventBuilder<'a> {
    pub(crate) fn new(client: &'a mut Client, event: &'a str, namespace: Option<&'a str>) -> Self {
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
            callback: self.callback,
            builder,
        }
    }
}
