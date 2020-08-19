use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use socket_io_protocol::socket::Args;

use super::AckBuilder;

// TODO: Is there a cleaner way to do this?
macro_rules! impl_fnmut_callback {
    ($(#[$attr:meta])* $name:ident ( $($arg:ident : $ty:ty),* )) => {
        $(#[$attr])*
        #[derive(Clone)]
        pub struct $name(Arc<Mutex<dyn 'static + Send + FnMut($($ty),*)>>);

        impl $name {
            pub fn call(&mut self, $($arg : $ty),*) {
                (&mut *self.0.lock().unwrap())($($arg),*)
            }
        }

        impl<F> From<F> for $name
        where
            F: 'static + Send + FnMut($($ty),*)
        {
            fn from(f: F) -> Self {
                $name(Arc::new(Mutex::new(f)))
            }
        }

        #[cfg(test)]
        impl From<Arc<Mutex<dyn 'static + Send + FnMut($($ty),*)>>> for $name {
            fn from(a: Arc<Mutex<dyn 'static + Send + FnMut($($ty),*)>>) -> Self {
                $name(a)
            }
        }
    }
}

macro_rules! impl_fnonce_callback {
    ($(#[$attr:meta])* $name:ident ( $($arg:ident : $ty:ty),* )) => {
        $(#[$attr])*
        pub struct $name(Box<dyn 'static + Send + FnOnce($($ty),*)>);

        impl $name {
            pub fn call(self, $($arg : $ty),*) {
                (self.0)($($arg),*)
            }
        }

        impl<F> From<F> for $name
        where
            F: 'static + Send + FnOnce($($ty),*)
        {
            fn from(f: F) -> Self {
                $name(Box::new(f))
            }
        }
    }
}

// TODO: Add callback for connection status updates

impl_fnmut_callback! {
    /// A wrapper type for event callbacks, which must be stored and called potentially repeatedly.
    /// They are stored as Arc<Mutex<dyn T>> to allow releasing the mutex on the main map of
    /// callbacks before calling the callback.
    EventCallback(args: &Args, ack: Option<AckBuilder>) // TODO: Add response builder
}

impl_fnonce_callback! {
    /// A wrapper type for ack callbacks, which only need to be called once.
    AckCallback(args: &Args)
}

pub struct Callbacks {
    namespaces: HashMap<String, Namespace>,
}

struct Namespace {
    fallback: Option<EventCallback>,
    events: HashMap<String, EventCallback>,
    acks: HashMap<u64, AckCallback>,
}

impl Callbacks {
    pub fn new() -> Self {
        Callbacks {
            namespaces: HashMap::new(),
        }
    }

    pub fn get_event(&self, namespace: &str, event: &str) -> Option<EventCallback> {
        let ns = self.namespaces.get(namespace)?;
        ns.events
            .get(event)
            .or(ns.fallback.as_ref())
            .map(EventCallback::clone)
    }

    pub fn set_event(&mut self, namespace: &str, event: &str, callback: impl Into<EventCallback>) {
        self.get_or_create_namespace(namespace)
            .events
            .insert(event.to_string(), callback.into());
    }

    pub fn clear_event(&mut self, namespace: &str, event: &str) {
        if let Some(ns) = self.namespaces.get_mut(namespace) {
            ns.events.remove(event);
        }
    }

    pub fn set_fallback(&mut self, namespace: &str, callback: impl Into<EventCallback>) {
        self.get_or_create_namespace(namespace).fallback = Some(callback.into());
    }

    pub fn clear_fallback(&mut self, namespace: &str) {
        if let Some(ns) = self.namespaces.get_mut(namespace) {
            ns.fallback = None;
        }
    }

    pub fn get_and_clear_ack(&mut self, namespace: &str, id: u64) -> Option<AckCallback> {
        let ns = self.namespaces.get_mut(namespace)?;
        ns.acks.remove(&id)
    }

    pub fn set_ack(&mut self, namespace: &str, id: u64, callback: impl Into<AckCallback>) {
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

        let c0: EventCallback = (|_args: &Args| {}).into();
        let c1: EventCallback = (|_args: &Args| {}).into();
        let c2: AckCallback = (|_args: &Args| {}).into();
        callbacks.set_event("/", "msg", c0.clone());
        callbacks.set_fallback("/", c1.clone());
        callbacks.set_ack("/", 0, c2);

        assert!(Arc::ptr_eq(
            &callbacks.get_event("/", "msg").as_ref().unwrap().0,
            &c0.0
        ));
        assert!(Arc::ptr_eq(
            &callbacks.get_event("/", "other").as_ref().unwrap().0,
            &c1.0
        ));
        assert!(callbacks.get_event("/ns", "msg").is_none());
        assert!(callbacks.get_and_clear_ack("/", 0).is_some());
        assert!(callbacks.get_and_clear_ack("/", 0).is_none());
    }
}
