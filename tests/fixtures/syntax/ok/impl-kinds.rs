impl S {}
impl<T: Clone> Trait for Vec<T> where T: Default {}
unsafe impl Send for S {}
impl !Sync for S {}
impl<'a> From<&'a str> for S {
    fn from(s: &'a str) -> Self {
        S
    }
}
impl <T as Trait>::Assoc {}
