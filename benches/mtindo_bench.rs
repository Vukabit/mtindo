use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mtindo::{reorganize_file, LayoutConfig};

fn bench_reorganize(c: &mut Criterion) {
    let source = generate_monolith();
    let config = LayoutConfig::default();

    c.bench_function("reorganize_10000_structs", |b| {
        b.iter(|| reorganize_file(black_box(source.clone()), black_box(&config), false))
    });
}

fn generate_monolith() -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < 10000 {
        let struct_def = format!("pub struct TestStruct{} {{ val: usize }}\n", i);
        out.push_str(&struct_def);
        i += 1;
    }
    out
}

criterion_group!(benches, bench_reorganize);

criterion_main!(benches);
