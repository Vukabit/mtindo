// edition: 2021
fn f() {
    let a = |x| x + 1;
    let b = || {};
    let c = move |a: u8, b| -> u8 { a };
    let d = async move || {};
    let e = for<'a> |x: &'a u8| x;
    let g = |(a, b): (u8, u8), _| a;
}
