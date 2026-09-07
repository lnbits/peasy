//! Native operation lifetime. Never compiled into the zero-import Wasm engine.
use std::cell::RefCell;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[derive(Clone, Default, Debug)]
pub struct Cancellation(Arc<AtomicU8>);

#[derive(Debug, thiserror::Error)]
#[error("Operation cancelled")]
pub struct Cancelled;

thread_local! {
    static CURRENT: RefCell<Option<Cancellation>> = const { RefCell::new(None) };
}

impl Cancellation {
    pub fn current() -> Self {
        CURRENT.with(|slot| slot.borrow().clone().unwrap_or_default())
    }

    /// False means a non-interruptible action already began. Cancellation and
    /// protection compete in one atomic transition, not a check-then-act race.
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| true)
            .unwrap_or_else(|state| state == 1)
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 1
    }

    pub fn is_protected(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 2
    }

    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn protect(&self) -> Result<(), Cancelled> {
        self.0
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .or_else(|state| if state == 2 { Ok(()) } else { Err(Cancelled) })
    }

    pub fn scope<T>(&self, work: impl FnOnce() -> T) -> T {
        struct Restore(Option<Cancellation>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(CURRENT.with(|slot| slot.replace(Some(self.clone()))));
        work()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_activation_are_mutually_exclusive() {
        for _ in 0..100 {
            let token = Cancellation::default();
            let other = token.clone();
            let cancelling = std::thread::spawn(move || other.cancel());
            let protected = token.protect().is_ok();
            assert_ne!(cancelling.join().unwrap(), protected);
        }
    }
    #[test]
    fn scopes_restore_the_previous_operation() {
        let outer = Cancellation::default();
        outer.scope(|| {
            Cancellation::default().scope(|| {
                Cancellation::current().cancel();
            });
            assert!(!Cancellation::current().is_cancelled());
            outer.cancel();
            assert!(Cancellation::current().check().is_err());
        });
        assert!(Cancellation::current().check().is_ok());
    }
}
