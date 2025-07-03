#![allow(clippy::incompatible_msrv)]

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::OnceCell;
use core::cell::RefCell;
use core::future::Future;
use core::mem::ManuallyDrop;
use core::pin::Pin;
use core::sync::atomic::AtomicI32;
use core::sync::atomic::Ordering::SeqCst;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use wasm_bindgen::prelude::*;

use super::Task as _;

const SLEEPING: i32 = 0;
const AWAKE: i32 = 1;

struct AtomicWaker {
    state: AtomicI32,
}

impl AtomicWaker {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: AtomicI32::new(AWAKE),
        })
    }

    fn wake_by_ref(&self) {
        // If we're already AWAKE then we previously notified and there's
        // nothing to do...
        match self.state.swap(AWAKE, SeqCst) {
            AWAKE => return,
            other => debug_assert_eq!(other, SLEEPING),
        }

        // ... otherwise we execute the native `notify` instruction to wake up
        // the corresponding `waitAsync` that was waiting for the transition
        // from SLEEPING to AWAKE.
        unsafe {
            core::arch::wasm32::memory_atomic_notify(
                self.state.as_ptr(),
                1, // Number of threads to notify
            );
        }
    }

    /// Same as the singlethread module, this creates a standard library
    /// `RawWaker`. We could use `futures_util::task::ArcWake` but it's small
    /// enough that we just inline it for now.
    unsafe fn into_raw_waker(this: Arc<Self>) -> RawWaker {
        unsafe fn raw_clone(ptr: *const ()) -> RawWaker {
            let ptr = ManuallyDrop::new(Arc::from_raw(ptr as *const AtomicWaker));
            AtomicWaker::into_raw_waker((*ptr).clone())
        }

        unsafe fn raw_wake(ptr: *const ()) {
            let ptr = Arc::from_raw(ptr as *const AtomicWaker);
            AtomicWaker::wake_by_ref(&ptr);
        }

        unsafe fn raw_wake_by_ref(ptr: *const ()) {
            let ptr = ManuallyDrop::new(Arc::from_raw(ptr as *const AtomicWaker));
            AtomicWaker::wake_by_ref(&ptr);
        }

        unsafe fn raw_drop(ptr: *const ()) {
            drop(Arc::from_raw(ptr as *const AtomicWaker));
        }

        const VTABLE: RawWakerVTable =
            RawWakerVTable::new(raw_clone, raw_wake, raw_wake_by_ref, raw_drop);

        RawWaker::new(Arc::into_raw(this) as *const (), &VTABLE)
    }
}

struct Inner {
    future: Pin<Box<dyn Future<Output = ()> + 'static>>,
    closure: Closure<dyn FnMut(JsValue)>,
}

pub(crate) struct Task {
    atomic: Arc<AtomicWaker>,
    waker: Waker,
    // See `singlethread.rs` for why this is an internal `Option`.
    inner: RefCell<Option<Inner>>,
}

impl Task {
    pub(crate) fn spawn(future: Pin<Box<dyn Future<Output = ()> + 'static>>) {
        let atomic = AtomicWaker::new();
        let waker = unsafe { Waker::from_raw(AtomicWaker::into_raw_waker(atomic.clone())) };
        let this = Rc::new(Task {
            atomic,
            waker,
            inner: RefCell::new(None),
        });

        let closure = {
            let this = Rc::clone(&this);
            Closure::new(move |_| this.run())
        };
        *this.inner.borrow_mut() = Some(Inner { future, closure });

        // Queue up the Future's work to happen on the next microtask tick.
        crate::queue::Queue::<Self>::with(move |queue| queue.schedule_task(this));
    }
}

impl super::Task for Task {
    fn run(&self) {
        let mut borrow = self.inner.borrow_mut();

        // Same as `singlethread.rs`, handle spurious wakeups happening after we
        // finished.
        let inner = match borrow.as_mut() {
            Some(inner) => inner,
            None => return,
        };

        loop {
            // Also the same as `singlethread.rs`, flag ourselves as ready to
            // receive a notification.
            let prev = self.atomic.state.swap(SLEEPING, SeqCst);

            if prev != AWAKE && !is_safari() {
                eprintln!(
                    "spurious wakeup from Atomics.waitAsync with prev={prev} (should be {AWAKE})"
                );
            }

            let poll = {
                let mut cx = Context::from_waker(&self.waker);
                inner.future.as_mut().poll(&mut cx)
            };

            match poll {
                // Same as `singlethread.rs` (noticing a pattern?) clean up
                // resources associated with the future ASAP.
                Poll::Ready(()) => {
                    *borrow = None;
                }

                // Unlike `singlethread.rs` we are responsible for ensuring there's
                // a closure to handle the notification that a Future is ready. In
                // the single-threaded case the notification itself enqueues work,
                // but in the multithreaded case we don't know what thread a
                // notification comes from so we need to ensure the current running
                // thread is the one that enqueues the work. To do that we execute
                // `Atomics.waitAsync`, creating a local Promise on our own thread
                // which will resolve once `Atomics.notify` is called.
                //
                // We could be in one of two states as we execute this:
                //
                // * `SLEEPING` - we'll get notified via `Atomics.notify`
                //   and then this Promise will resolve.
                //
                // * `AWAKE` - the Promise will immediately be resolved and
                //   we'll execute the work on the next microtask queue.
                Poll::Pending => {
                    match wait_async(&self.atomic.state, SLEEPING) {
                        Some(promise) => drop(promise.then(&inner.closure)),
                        // our state has already changed so we can just do the work
                        // again inline.
                        None => continue,
                    }
                }
            }
            break;
        }
    }
}

#[wasm_bindgen]
extern "C" {
    type Atomics;
    type WaitAsyncResult;

    #[wasm_bindgen(static_method_of = Atomics, js_name = waitAsync)]
    fn wait_async(buf: &js_sys::Int32Array, index: u32, value: i32) -> WaitAsyncResult;

    #[wasm_bindgen(static_method_of = Atomics, js_name = waitAsync, getter)]
    fn get_wait_async() -> JsValue;

    #[wasm_bindgen(method, getter, structural, js_name = async)]
    fn async_(this: &WaitAsyncResult) -> bool;

    #[wasm_bindgen(method, getter, structural)]
    fn value(this: &WaitAsyncResult) -> js_sys::Promise;
}

fn user_agent() -> String {
    let global = js_sys::global();
    let navigator = js_sys::Reflect::get(&global, &JsValue::from_str("navigator"))
        .ok()
        .and_then(|nav| nav.dyn_into::<js_sys::Object>().ok());
    navigator
        .as_ref()
        .and_then(|nav| js_sys::Reflect::get(nav, &JsValue::from_str("userAgent")).ok())
        .and_then(|ua| ua.as_string())
        .unwrap_or_default()
}

fn is_safari() -> bool {
    #[thread_local]
    static IS_SAFARI: OnceCell<bool> = OnceCell::new();
    *IS_SAFARI.get_or_init(|| {
        let user_agent = user_agent();
        user_agent.contains("Safari") && !user_agent.contains("Chrome")
    })
}

fn is_wait_async_available() -> bool {
    #[thread_local]
    static AVAILABLE: OnceCell<bool> = OnceCell::new();
    *AVAILABLE.get_or_init(|| {
        option_env!("WBG_DISABLE_ATOMICS_WAIT_ASYNC").is_none()
            && !Atomics::get_wait_async().is_undefined()
            && !is_safari()
    })
}

fn wait_async(ptr: &AtomicI32, current_value: i32) -> Option<js_sys::Promise> {
    if !is_wait_async_available() {
        if ptr.load(SeqCst) == current_value {
            let timeout = match is_safari() {
                true => Some(1000),
                false => None,
            };
            Some(crate::task::wait_async_polyfill::wait_async(
                ptr,
                current_value,
                timeout,
            ))
        } else {
            None
        }
    } else {
        let mem = wasm_bindgen::memory().unchecked_into::<js_sys::WebAssembly::Memory>();
        let array = js_sys::Int32Array::new(&mem.buffer());
        let result = Atomics::wait_async(&array, ptr.as_ptr() as u32 / 4, current_value);
        if result.async_() {
            Some(result.value())
        } else {
            None
        }
    }
}
