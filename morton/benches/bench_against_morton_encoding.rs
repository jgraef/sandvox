use std::hint::black_box;

use criterion::{
    Criterion,
    criterion_group,
    criterion_main,
};
use morton::Morton;

fn bench_intrinsics_against_morton_encoding(c: &mut Criterion) {
    {
        let data: [u16; 2] = [64402, 690];

        let mut group = c.benchmark_group("encode_u16_2");

        group.bench_function("intrinsics", |b| b.iter(|| black_box(data).morton_encode()));

        group.bench_function("morton_encoding", |b| {
            b.iter(|| morton_encoding::morton_encode(black_box(data)))
        });

        group.finish();
    }

    {
        let data: [u16; 3] = [64402, 690, 14508];

        let mut group = c.benchmark_group("encode_u16_3");

        group.bench_function("intrinsics", |b| b.iter(|| black_box(data).morton_encode()));

        group.bench_function("morton_encoding", |b| {
            b.iter(|| morton_encoding::morton_encode(black_box(data)))
        });

        group.finish();
    }
}

criterion_group!(benches, bench_intrinsics_against_morton_encoding);
criterion_main!(benches);
