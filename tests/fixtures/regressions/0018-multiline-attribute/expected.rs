#[derive(
    Debug,
    Clone
)]
pub struct Z;

fn a() {}

#[cfg(
    any(unix, windows)
)]
fn b() {}
