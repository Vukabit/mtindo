fn f() -> u8 {
    if a {
    } else if let Some(x) = b && x > 1 {
    } else {
    }
    while let Some(x) = it.next() {}
    'outer: loop {
        break 'outer 3;
        continue 'outer;
    }
    let v = 'block: {
        break 'block 1;
    };
    match x {
        A | B if ready => 1,
        C => {}
        _ => return 2,
    }
    return 0;
}
