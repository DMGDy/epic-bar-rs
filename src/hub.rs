use std::{cell::RefCell, rc::Rc};

/* A latest-value broadcaster for the GTK main thread. Every output gets its
 * own set of bars, but each data source (status sampling, audio, GPU...)
 * runs once and publishes here. Subscribers return false once their widgets
 * are gone, so bars rebuilt after an output hotplug do not leak listeners.
 */
type Listener<T> = Box<dyn Fn(&T) -> bool>;

pub struct Hub<T> {
    value: RefCell<Option<T>>,
    listeners: RefCell<Vec<Listener<T>>>,
}

impl<T> Default for Hub<T> {
    fn default() -> Self {
        Self {
            value: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
        }
    }
}

impl<T: 'static> Hub<T> {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    pub fn publish(&self, value: T) {
        // Listeners may subscribe or read the value; never hold a borrow
        // across their calls.
        let listeners = self.listeners.take();
        let kept: Vec<_> = listeners
            .into_iter()
            .filter(|listener| listener(&value))
            .collect();
        let mut current = self.listeners.borrow_mut();
        let added = std::mem::replace(&mut *current, kept);
        current.extend(added);
        drop(current);
        *self.value.borrow_mut() = Some(value);
    }

    /* Call `listener` with the current value (if any) and every later one. */
    pub fn subscribe(&self, listener: impl Fn(&T) -> bool + 'static) {
        if let Some(value) = &*self.value.borrow() {
            if !listener(value) {
                return;
            }
        }
        self.listeners.borrow_mut().push(Box::new(listener));
    }

    pub fn with<R>(&self, read: impl FnOnce(Option<&T>) -> R) -> R {
        read(self.value.borrow().as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn subscribers_get_current_and_later_values_until_they_stop() {
        let hub = Hub::new();
        hub.publish(1);
        let seen = Rc::new(Cell::new(0));
        let calls = Rc::new(Cell::new(0));
        let (seen_in, calls_in) = (seen.clone(), calls.clone());
        hub.subscribe(move |value| {
            seen_in.set(*value);
            calls_in.set(calls_in.get() + 1);
            *value < 3
        });
        assert_eq!(seen.get(), 1);
        hub.publish(3);
        assert_eq!(seen.get(), 3);
        hub.publish(4);
        assert_eq!(seen.get(), 3, "listener returned false and must be dropped");
        assert_eq!(calls.get(), 2);
        assert_eq!(hub.with(|value| value.copied()), Some(4));
    }
}
