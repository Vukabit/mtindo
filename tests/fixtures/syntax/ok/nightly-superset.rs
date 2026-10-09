// edition: 2024
pub impl(crate) trait Sealed {}
pub impl(in crate::a) const unsafe trait Restricted {}
struct S {
    pub mut(crate) field: u8,
}
trait T {
    final fn fixed() {}
}
impl Ty {
    reuse std::convert::identity;
}
const {
    assert!(true);
}
type U = unsafe<'a> &'a ();
type P = builtin # field_of(Ty, field);
fn bounds<T: [const] Drop>(f: impl Fn(msg: String)) where T::method(..): Send {}
fn exprs(x: &pin mut u8) {
    super let y = 1;
    let z = try bikeshed Option<u8> { 4 };
    for await item in stream {}
    let c = x.use;
    let cl = use || 1;
    let m = v.match {
        _ => 1,
    };
    let b = builtin # offset_of(Foo, bar);
    let g = async gen move {};
}
fn patterns(&pin mut x: &pin mut u8, ref pin mut y: u8, mut ref mut z: u8) {
    match v {
        (Plan::A if ok) | (Plan::B if fine) => {}
        _ => {}
    }
}
