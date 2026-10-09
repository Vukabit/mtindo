fn f(c: char) -> bool {
    (c as u8 >= 0x41 && c as u8 <= 0x5A) || c as usize > 3
}
