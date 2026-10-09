impl S {
    fn a(self) {}
    fn b(mut self, x: u8) {}
    fn c(&self) {}
    fn d(&'a mut self) {}
    fn e(self: Box<Self>) {}
    fn f(&mut self, (a, b): (u8, u8), _: u8, Foo { x, .. }: Foo, E::A(y): E) {}
}
