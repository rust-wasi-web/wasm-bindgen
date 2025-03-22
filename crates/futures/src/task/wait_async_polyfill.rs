//!
//! The polyfill was kindly borrowed from https://github.com/tc39/proposal-atomics-wait-async
//! and ported to Rust
//!

#![allow(clippy::incompatible_msrv)]

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 *
 * Author: Lars T Hansen, lhansen@mozilla.com
 */

/* Polyfill for Atomics.waitAsync() for web browsers.
 *
 * Any kind of agent that is able to create a new Worker can use this polyfill.
 *
 * Load this file in all agents that will use Atomics.waitAsync.
 *
 * Agents that don't call Atomics.waitAsync need do nothing special.
 *
 * Any kind of agent can wake another agent that is sleeping in
 * Atomics.waitAsync by just calling Atomics.wake for the location being slept
 * on, as normal.
 *
 * The implementation is not completely faithful to the proposed semantics: in
 * the case where an agent first asyncWaits and then waits on the same location:
 * when it is woken, the two waits will be woken in order, while in the real
 * semantics, the sync wait will be woken first.
 *
 * In this polyfill Atomics.waitAsync is not very fast.
 */

/* Implementation:
 *
 * For every wait we fork off a Worker to perform the wait.  Workers are reused
 * when possible.  The worker communicates with its parent using postMessage.
 */

use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;
use core::cell::RefCell;
use core::sync::atomic::AtomicI32;
use js_sys::{Array, Promise};
use std::rc::Rc;
use std::thread;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, Worker, WorkerOptions};

/// Number of web worker helpers to keep alive.
const HELPER_CACHE_SIZE: usize = 32;

/// A worker that is terminated when dropped.
struct WorkerGuard(Worker);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.terminate();
    }
}

#[thread_local]
static HELPERS: RefCell<Vec<Rc<WorkerGuard>>> = RefCell::new(vec![]);

#[thread_local]
static ID: RefCell<usize> = RefCell::new(0);

fn alloc_helper() -> Rc<WorkerGuard> {
    if let Some(helper) = HELPERS.borrow_mut().pop() {
        return helper;
    }

    let mut id = ID.borrow_mut();

    let opts = WorkerOptions::new();
    opts.set_name(&format!(
        "atomic-wait-async-{}{:?}-{id}",
        option_env!("WBG_PKG").unwrap_or_default(),
        thread::current().id(),
    ));

    let worker_url = wasm_bindgen::link_to!(module = "/src/task/worker.js");
    let worker = Worker::new_with_options(&worker_url, &opts)
        .unwrap_or_else(|js| wasm_bindgen::throw_val(js));

    *id = id.wrapping_add(1);

    Rc::new(WorkerGuard(worker))
}

fn free_helper(helper: Rc<WorkerGuard>) {
    let mut helpers = HELPERS.borrow_mut();
    if helpers.len() < HELPER_CACHE_SIZE {
        helpers.push(helper);
    }
}

const LONG_WAIT_WARNING: Option<u32> = match option_env!("WBG_LONG_WAIT_WARNING") {
    Some(t) => match u32::from_str_radix(t, 10) {
        Ok(t) => Some(t),
        Err(_) => None,
    },
    None => None,
};

pub fn wait_async(ptr: &AtomicI32, value: i32, timeout: Option<i32>) -> Promise {
    Promise::new(&mut |resolve, _reject| {
        let helper = alloc_helper();
        let helper_ref = helper.clone();

        let done = match LONG_WAIT_WARNING {
            Some(warning_timeout) => {
                let done = Rc::new(Cell::new(false));

                let timeout_closure = Closure::once_into_js({
                    let done = done.clone();
                    move || {
                        if !done.get() {
                            web_sys::console::trace_1(
                                &format!("spawned task wait duration exceeds {warning_timeout} ms")
                                    .into(),
                            );
                        }
                    }
                });

                let global = js_sys::global();
                let set_timeout = js_sys::Reflect::get(&global, &"setTimeout".into()).unwrap();
                let set_timeout_fn = set_timeout.dyn_into::<js_sys::Function>().unwrap();
                set_timeout_fn
                    .call2(
                        &global,
                        timeout_closure.as_ref().unchecked_ref(),
                        &JsValue::from_f64(warning_timeout as f64),
                    )
                    .unwrap();

                Some(done)
            }
            None => None,
        };

        let onmessage_callback = Closure::once_into_js(move |e: MessageEvent| {
            // Our helper is done waiting so it's available to wait on a
            // different location, so return it to the free list.
            free_helper(helper_ref);
            drop(resolve.call1(&JsValue::NULL, &e.data()));

            if let Some(done) = done {
                done.set(true);
            }
        });
        helper
            .0
            .set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));

        let timeout = match timeout {
            Some(timeout) => JsValue::from(timeout),
            None => JsValue::undefined(),
        };

        let data = Array::of4(
            &wasm_bindgen::memory(),
            &JsValue::from(ptr.as_ptr() as u32 / 4),
            &JsValue::from(value),
            &timeout,
        );

        helper
            .0
            .post_message(&data)
            .unwrap_or_else(|js| wasm_bindgen::throw_val(js));
    })
}
