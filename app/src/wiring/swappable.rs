//! A shared adapter that can be swapped for another while others keep using it.
use std::sync::{Arc, RwLock};

/// Holds the adapter in use. Callers take the current one for each piece of work, so a
/// replacement applies from the next piece of work on. A panic while the lock was held
/// does not break it: the last stored adapter stays usable.
pub struct Swappable<T: ?Sized>(RwLock<Arc<T>>);

impl<T: ?Sized> Swappable<T> {
    pub fn new(initial: Arc<T>) -> Self {
        Self(RwLock::new(initial))
    }

    /// Uses `next` from now on.
    pub fn replace(&self, next: Arc<T>) {
        match self.0.write() {
            Ok(mut guard) => *guard = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
    }

    /// The adapter in use right now.
    pub fn current(&self) -> Arc<T> {
        match self.0.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replacement_is_used_from_then_on() {
        let s: Swappable<str> = Swappable::new(Arc::from("old"));
        assert_eq!(&*s.current(), "old");

        s.replace(Arc::from("new"));

        assert_eq!(&*s.current(), "new");
    }

    #[test]
    fn a_panic_while_swapping_leaves_it_usable() {
        let s: Swappable<str> = Swappable::new(Arc::from("old"));
        std::thread::scope(|scope| {
            let crashed = scope
                .spawn(|| {
                    let _held = s.0.write().unwrap();
                    panic!("crash while holding the lock");
                })
                .join();
            assert!(crashed.is_err());
        });
        assert!(s.0.is_poisoned());

        assert_eq!(&*s.current(), "old");
        s.replace(Arc::from("new"));
        assert_eq!(&*s.current(), "new");
    }
}
