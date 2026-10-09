mod inner {}

fn a() {}

fn b() {}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}
