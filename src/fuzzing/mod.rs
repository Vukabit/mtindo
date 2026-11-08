//! The fuzz targets' checks, one module per target in `fuzz/fuzz_targets/`, each a
//! `check(data)` that panics on a violation. The targets call them, and so does
//! `tests/fuzz_regressions.rs` for every input a fuzz run found and that was kept as a
//! test, so a fixed crash stays fixed under exactly the checks that found it.

pub mod comments;
pub mod format;
pub mod generate;
pub mod generated;
pub mod layout;
pub mod lexer;
pub mod parser;
pub mod toml;
pub mod verify;

use std::panic;
use std::sync::{Mutex, OnceLock, mpsc};

/// The thread the checks run on, made once: with the stack mto's binaries give this work
/// ([`crate::STACK`]), which a fuzzer's own thread may not have. One thread for every
/// input, since a sanitizer keeps memory for each thread's stack long after it ends.
static WORKER: OnceLock<Mutex<(mpsc::Sender<Job>, mpsc::Receiver<Done>)>> = OnceLock::new();

/// How a check on the worker ended: a panic's payload, to raise again.
type Done = std::thread::Result<()>;

/// A check with its input, for the worker.
type Job = (Vec<u8>, fn(&[u8]));

/// Runs `check` on `data` on the worker, and waits for it. Under a fuzzer a panic there ends
/// the process, as its hook makes every panic do; elsewhere the worker catches it and this
/// raises it again on the caller's thread, so the worker lives on for the next input.
fn on_stack(data: &[u8], check: fn(&[u8])) {
  let worker = WORKER.get_or_init(|| {
    let (jobs, inbox) = mpsc::channel::<Job>();
    let (done, finished) = mpsc::channel();
    std::thread::Builder::new()
      .stack_size(crate::STACK)
      .spawn(move || {
        for (data, check) in inbox {
          let _ = done.send(panic::catch_unwind(|| check(&data)));
        }
      })
      .expect("a thread to run on");
    Mutex::new((jobs, finished))
  });
  let worker = worker
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  worker.0.send((data.to_vec(), check)).expect("the worker");
  if let Err(payload) = worker.1.recv().expect("the worker") {
    panic::resume_unwind(payload);
  }
}
