fn f() {
    match v {
        0 | -1 => {}
        1..=5 => {}
        'a'..='z' => {}
        ..=10 => {}
        LIMIT.. => {}
        _ => {}
        ref mut x => {}
        n @ 1..=9 => {}
        Some(inner) => {}
        Point { x, y: 0, ref mut z, .. } => {}
        E::A(..) => {}
        <T as Trait>::C => {}
        (a, .., b) => {}
        [first, rest @ .., last] => {}
        &(a, b) => {}
        &&x => {}
        const { 1 + 2 } => {}
        m!() => {}
        (A | B) => {}
    }
    let (a, b) = pair;
    let Some(x) = opt else { return };
}
