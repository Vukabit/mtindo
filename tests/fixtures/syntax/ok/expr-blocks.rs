// edition: 2021
fn f() {
    unsafe { g() }
    let a = const { 1 + 1 };
    let b = async { 1 };
    let c = async move { 2 };
    let d = { 3 };
}
