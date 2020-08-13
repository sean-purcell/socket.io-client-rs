use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use socket_io_protocol::socket::Args;

// TODO: Is there a cleaner way to do this?
#[derive(Clone)]
pub struct Callback(
    Arc<
        Mutex<
            dyn 'static
                + for<'a> FnMut(
                    &'a Args<'a>, // TODO: Add ack callback here once sending exists
                )
                + Send,
        >,
    >,
);

pub struct Callbacks {
    namespaces: HashMap<String, Namespace>,
}

struct Namespace {
    fallback: Option<Callback>,
    events: HashMap<String, Callback>,
    acks: HashMap<u64, Callback>,
}

impl<F> From<F> for Callback
where
    F: 'static + for<'a> FnMut(&'a Args<'a>) + Send,
{
    fn from(f: F) -> Callback {
        Callback(Arc::new(Mutex::new(f)))
    }
}

impl From<Arc<Mutex<dyn 'static + for<'a> FnMut(&'a Args<'a>) + Send>>> for Callback {
    fn from(a: Arc<Mutex<dyn 'static + for<'a> FnMut(&'a Args<'a>) + Send>>) -> Callback {
        Callback(a)
    }
}

impl Callback {
    pub fn call<'a>(&self, args: &'a Args<'a>) {
        let mut guard = self.0.lock().unwrap();
        (&mut *guard)(args)
    }
}

impl Callbacks {
    pub fn new() -> Self {
        Callbacks {
            namespaces: HashMap::new(),
        }
    }

    pub fn get_event(&self, namespace: &str, event: &str) -> Option<Callback> {
        let ns = self.namespaces.get(namespace)?;
        ns.events
            .get(event)
            .or(ns.fallback.as_ref())
            .map(Callback::clone)
    }

    pub fn set_event(&mut self, namespace: &str, event: &str, callback: impl Into<Callback>) {
        self.get_or_create_namespace(namespace)
            .events
            .insert(event.to_string(), callback.into());
    }

    pub fn clear_event(&mut self, namespace: &str, event: &str) {
        if let Some(ns) = self.namespaces.get_mut(namespace) {
            ns.events.remove(event);
        }
    }

    pub fn set_fallback(&mut self, namespace: &str, callback: impl Into<Callback>) {
        self.get_or_create_namespace(namespace).fallback = Some(callback.into());
    }

    pub fn clear_fallback(&mut self, namespace: &str) {
        if let Some(ns) = self.namespaces.get_mut(namespace) {
            ns.fallback = None;
        }
    }

    pub fn get_and_clear_ack(&mut self, namespace: &str, id: u64) -> Option<Callback> {
        let ns = self.namespaces.get_mut(namespace)?;
        ns.acks.remove(&id)
    }

    pub fn set_ack(&mut self, namespace: &str, id: u64, callback: impl Into<Callback>) {
        self.get_or_create_namespace(namespace)
            .acks
            .insert(id, callback.into());
    }

    fn get_or_create_namespace(&mut self, namespace: &str) -> &mut Namespace {
        self.namespaces
            .entry(namespace.to_string())
            .or_insert_with(Namespace::new)
    }
}

impl Namespace {
    fn new() -> Self {
        Namespace {
            fallback: None,
            events: HashMap::new(),
            acks: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple() {
        let mut callbacks = Callbacks::new();

        let c0: Callback = (|_args: &Args| {}).into();
        let c1: Callback = (|_args: &Args| {}).into();
        let c2: Callback = (|_args: &Args| {}).into();
        callbacks.set_event("/", "msg", c0.clone());
        callbacks.set_fallback("/", c1.clone());
        callbacks.set_ack("/", 0, c2.clone());

        assert!(Arc::ptr_eq(
            &callbacks.get_event("/", "msg").as_ref().unwrap().0,
            &c0.0
        ));
        assert!(Arc::ptr_eq(
            &callbacks.get_event("/", "other").as_ref().unwrap().0,
            &c1.0
        ));
        assert!(callbacks.get_event("/ns", "msg").is_none());
        assert!(Arc::ptr_eq(
            &callbacks.get_and_clear_ack("/", 0).as_ref().unwrap().0,
            &c2.0
        ));
        assert!(callbacks.get_and_clear_ack("/", 0).is_none());
    }
}
