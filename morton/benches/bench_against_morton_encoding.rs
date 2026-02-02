use std::hint::black_box;

use criterion::{
    Criterion,
    criterion_group,
    criterion_main,
};

fn encode2(c: &mut Criterion) {
    let data: [u16; 2] = [64402, 690];

    let mut group = c.benchmark_group("encode2");

    group.bench_function("intrinsics", |b| {
        b.iter(|| morton::encode2(black_box(data)))
    });

    group.bench_function("morton_encoding", |b| {
        b.iter(|| morton_encoding::morton_encode(black_box(data)))
    });

    group.finish();
}

criterion_group!(benches, encode2);
criterion_main!(benches);
