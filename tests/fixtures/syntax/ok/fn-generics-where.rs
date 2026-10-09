fn f<'a, 'b: 'a, T: Clone + 'a, U = u8, const N: usize, const M: bool = true>(t: &'a T) -> U
where
    T: Iterator<Item = U> + ?Sized,
    for<'x> F: Fn(&'x u8) -> &'x u8,
    'b: 'a,
{
    todo!()
}
