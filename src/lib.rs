//! Thread-local storage with guarded scopes.
//!
//! This crate provides thread-local variables whose values can be temporarily
//! overridden within a scope. Each time you call [set](GuardedKey::set), a new
//! value is pushed onto the thread-local stack, and a [Guard] is returned.
//! When the guard is dropped, the associated value is removed from the stack.
//! This enables safe, nested overrides of thread-local state.
//!
//! # Usage
//!
//! Use the [guarded_thread_local] macro to define a thread-local key. Call
//! [set](GuardedKey::set) to override the value for the current thread and
//! receive a guard. The value is accessible via [get](GuardedKey::get) while
//! the guard is alive.
//!
//! ```
//! use guarded_tls::guarded_thread_local;
//!
//! guarded_thread_local!(static FOO: String);
//!
//! let _guard1 = FOO.set("abc".into());
//! assert_eq!(FOO.get(), "abc");
//!
//! let guard2 = FOO.set("def".into());
//! assert_eq!(FOO.get(), "def");
//!
//! drop(guard2);
//! assert_eq!(FOO.get(), "abc");
//! ```
//!
//! # Notes
//!
//! - [get](GuardedKey::get) requires the value type to implement [Clone].
//! - Accessing the value without having a guard will panic.
//! - Guards dropped out of order have well-defined behavior.
//!
//! # See Also
//!
//! - [scoped-tls](https://docs.rs/scoped-tls/): a similar crate for scoped
//!   thread-local values.
//!
//! The main difference between this crate and `scoped-tls` is that this crate
//! doesn't require the nesting of functions, making it some application easier
//! to manage. For instance creating a test fixture that holds a [Guard].
//!
//! ```
//! guarded_tls::guarded_thread_local!(static FOO: u32);
//!
//! # use guarded_tls::Guard;
//! # struct MyFixture { foo_guard: Guard<u32> }
//! fn create_fixture() -> MyFixture {
//!     MyFixture { foo_guard: FOO.set(123) }
//! }
//!
//! fn my_test() {
//!     let fixture = create_fixture();
//!
//!     // Test code here that assumes `FOO` is set.
//!     assert_eq!(FOO.get(), 123);
//! }
//!
//! my_test();
//! ```
use std::{cell::RefCell, thread::LocalKey};

#[macro_export]
macro_rules! guarded_thread_local {
    ($(#[$attrs:meta])* $vis:vis static $name:ident: $ty:ty) => (
        $(#[$attrs])*
        $vis static $name: $crate::GuardedKey<$ty> = {
            ::std::thread_local!(static FOO: ::std::cell::RefCell<$crate::Inner<$ty>> = const {
                ::std::cell::RefCell::new($crate::Inner::new())
            });
            $crate::GuardedKey::new(&FOO)
        };
    )
}

/// A nested thread-local that spawns a [Guard] for each [set](GuardedKey::set).
pub struct GuardedKey<T: 'static> {
    inner: &'static LocalKey<RefCell<Inner<T>>>,
}

impl<T: 'static> GuardedKey<T> {
    #[doc(hidden)]
    pub const fn new(inner: &'static LocalKey<RefCell<Inner<T>>>) -> Self {
        Self { inner }
    }

    /// Sets the value of this thread-local and returns a [Guard].
    ///
    /// After this call, [get](GuardedKey::get) will return the value that was
    /// provided here.
    #[must_use]
    pub fn set(&'static self, t: T) -> Guard<T> {
        self.inner.with_borrow_mut(move |inner| {
            inner.item.push(Some(t));
            Guard {
                inner: self.inner,
                index: inner.item.len() - 1,
            }
        })
    }
}

impl<T: Clone + 'static> GuardedKey<T> {
    /// Clones and returns the last value of thread-local stack.
    ///
    /// # Panics
    ///
    /// Panics if this thread-local has not previously been
    /// [set](GuardedKey::set).
    ///
    /// Panics if the [Clone] implementation of `T` accesses this same thread
    /// local.
    pub fn get(&'static self) -> T {
        let Some(val) = self.inner.with_borrow(|inner| inner.item.last().cloned()) else {
            panic!("cannot access a guarded thread local variable without calling `set` first")
        };

        // The top of the stack cannot be None, as Guard::drop will pop from the stack
        // until it finds a non-None entry.
        val.expect("internal error: top of item list is none")
    }
}

#[doc(hidden)]
pub struct Inner<T: 'static> {
    item: Vec<Option<T>>,
}

impl<T: 'static> Inner<T> {
    #[doc(hidden)]
    pub const fn new() -> Self {
        Self { item: Vec::new() }
    }
}

/// Keeps a thread local value alive. Removes its associated value from the
/// stack upon being dropped.
pub struct Guard<T: 'static> {
    inner: &'static LocalKey<RefCell<Inner<T>>>,
    index: usize,
}

impl<T> Drop for Guard<T> {
    /// Removes associated value from the thread-local stack. If this is the
    /// last existing guard for this thread-local, then any
    /// subsequent [get](GuardedKey::get) will panic unless the thread-local
    /// is [set](GuardedKey::set) again.
    fn drop(&mut self) {
        self.inner.with_borrow_mut(|inner| {
            *inner.item.get_mut(self.index).unwrap() = None;

            while let Some(item) = inner.item.last() {
                if item.is_none() {
                    let _ = inner.item.pop();
                } else {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        guarded_thread_local!(static FOO: u32);
        let _foo_guard_1 = FOO.set(3);
        assert_eq!(FOO.get(), 3);
        assert_eq!(FOO.get(), 3);

        let foo_guard_2 = FOO.set(123);
        assert_eq!(FOO.get(), 123);

        drop(foo_guard_2);
        assert_eq!(FOO.get(), 3);
    }

    #[test]
    #[should_panic(
        expected = "cannot access a guarded thread local variable without calling `set` first"
    )]
    fn get_without_set() {
        guarded_thread_local!(static FOO: u32);
        let _ = FOO.get();
    }

    #[test]
    fn out_of_order_guard_drop() {
        guarded_thread_local!(static FOO: u32);
        let guard_1 = FOO.set(1);
        let guard_2 = FOO.set(2);
        let guard_3 = FOO.set(3);
        assert_eq!(FOO.get(), 3);

        drop(guard_1);
        assert_eq!(FOO.get(), 3);

        drop(guard_3);
        assert_eq!(FOO.get(), 2);

        drop(guard_2);
    }

    #[test]
    fn non_copy_type() {
        guarded_thread_local!(static FOO: String);
        let _guard_1 = FOO.set("x".into());
        let guard_2 = FOO.set("y".into());

        assert_eq!(FOO.get(), "y");
        drop(guard_2);
        assert_eq!(FOO.get(), "x");
    }

    #[test]
    #[should_panic(expected = "already borrowed: BorrowMutError")]
    fn clone_access_same_thread_local() {
        guarded_thread_local!(static FOO: X);

        struct X;

        impl Clone for X {
            fn clone(&self) -> Self {
                let _ = FOO.set(X);
                X
            }
        }

        let _guard = FOO.set(X);
        let _ = FOO.get();
    }
}
