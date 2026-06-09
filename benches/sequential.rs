use arctic::sequential;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn insert_u64_u64(criterion: &mut Criterion) {
    criterion.bench_function("insert sequential 1M", |bencher| {
        bencher.iter_with_large_drop(|| {
            let mut map = sequential::Map::<u64, u64>::new();

            for key in 0u64..1_000_000 {
                map.insert(key, key).unwrap();
            }
        })
    });
}

criterion_group!(name = insert; config = Criterion::default(); targets = insert_u64_u64);
criterion_main!(insert);
