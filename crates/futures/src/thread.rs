//! Thread holding.
//!
//! This module provides functions to hold and release the current thread
//! in a WebAssembly environment using the WASI (WebAssembly System Interface).
//!
//! Thread holding allows a thread to stay alive after it has returned from
//! its start function, i.e. the function passed to [`std::thread::spawn`].
//! This is useful in the web enviroment to return control to the event
//! loop of the executor and thus allow async callbacks and Promises
//! to run.
//!
//! To request thread holding call [`thread_hold`] during the execution of
//! its start function and return normally from the start function.
//!

#[link(wasm_import_module = "wasi")]
unsafe extern "C" {
    /// Holds the current thread after its start function has returned.
    ///
    /// This gives back control to the event loop of the executor and
    /// allow async callbacks and Promises to run.
    ///
    /// Call [`thread_release`] to cleanup and end the held thread.
    ///
    /// This function can only be called at most once per thread.
    #[link_name = "thread-hold"]
    pub fn thread_hold();

    /// Cleans up and exits the current thread that has previously been
    /// suspended using [`thread_hold`].
    ///
    /// The thread will exit once the current callback has completed
    /// and control is released back to the event loop.
    #[link_name = "thread-release"]
    pub fn thread_release();
}
