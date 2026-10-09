fn f() {
    use std::fmt;
    struct Local;
    fn helper() {}
    const LIMIT: u8 = 3;
    let c = const { 4 };
    #[cfg(test)]
    let a = 1;
    #[allow(unused)]
    g();
    let b = #[attr] {
        5
    };
}
