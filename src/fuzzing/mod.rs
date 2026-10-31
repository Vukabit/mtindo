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

use std::sync::{Mutex, OnceLock, mpsc};

/// The thread the checks run on, made once: with the stack mto's binaries give this work
/// ([`crate::STACK`]), which a fuzzer's own thread may not have. One thread for every
/// input, since a sanitizer keeps memory for each thread's stack long after it ends.
static WORKER: OnceLock<Mutex<(mpsc::Sender<Job>, mpsc::Receiver<()>)>> = OnceLock::new();

/// A check with its input, for the worker.
type Job = (Vec<u8>, fn(&[u8]));

/// Runs `check` on `data` on the worker, and waits for it. A panic there ends the process,
/// as the fuzzer's hook makes every panic do.
fn on_stack(data: &[u8], check: fn(&[u8])) {
  let worker = WORKER.get_or_init(|| {
    let (jobs, inbox) = mpsc::channel::<Job>();
    let (done, finished) = mpsc::channel();
    std::thread::Builder::new()
      .stack_size(crate::STACK)
      .spawn(move || {
        for (data, check) in inbox {
          check(&data);
          let _ = done.send(());
        }
      })
      .expect("a thread to run on");
    Mutex::new((jobs, finished))
  });
  let worker = worker
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  worker.0.send((data.to_vec(), check)).expect("the worker");
  worker.1.recv().expect("the worker");
}
