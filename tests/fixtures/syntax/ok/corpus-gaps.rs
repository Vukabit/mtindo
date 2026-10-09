// edition: 2024
// Valid syntax the corpus check found mto rejecting (stage 1, M7).
type GenericArgBounds = Foo<Item<T>: Bound, Item::<T>: Bound, Item(T): Bound, Item(..) = Item>;
type const FOO: i32 = 2;
gen fn numbers() {}
async gen fn more_numbers() {}
fn f() {
    x.self;
    x.Self;
    let builtin # deref(_) = 1;
    let _: Box<(?Sized) + (for<'a> Trait<'a>) + (Copy)>;
    gen fn inner() {}
    do yeet;
    do yeet 1
}
