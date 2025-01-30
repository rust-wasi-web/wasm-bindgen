use core::future::Future;
use core::pin::Pin;

pub(crate) mod singlethread;

#[cfg(target_feature = "atomics")]
pub(crate) mod multithread;
#[cfg(target_feature = "atomics")]
mod wait_async_polyfill;

pub(crate) trait Task {
    fn run(&self);
}

pub(crate) fn spawn(future: Pin<Box<dyn Future<Output = ()> + 'static>>) {
    #[cfg(target_feature = "atomics")]
    match threads_avilable() {
        false => singlethread::Task::spawn(future),
        true => multithread::Task::spawn(future),
    }

    #[cfg(not(target_feature = "atomics"))]
    singlethread::Task::spawn(future)
}

/// Whether threads are available in the current environment.
#[cfg(target_feature = "atomics")]
fn threads_avilable() -> bool {
    use core::sync::atomic::{AtomicU8, Ordering};
    use wasm_bindgen::JsValue;

    const UNKNOWN: u8 = 0;
    const AVAILABLE: u8 = 1;
    const UNAVAILABLE: u8 = 2;

    static VALUE: AtomicU8 = AtomicU8::new(UNKNOWN);

    match VALUE.load(Ordering::Relaxed) {
        UNKNOWN => (),
        AVAILABLE => return true,
        UNAVAILABLE => return false,
        _ => unreachable!(),
    }

    let avail = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("crossOriginIsolated"))
        .ok()
        .and_then(|obj| obj.as_bool())
        .unwrap_or_default();

    VALUE.store(
        if avail { AVAILABLE } else { UNAVAILABLE },
        Ordering::Relaxed,
    );

    avail
}
