use criterion::{criterion_group, criterion_main, Criterion};
use photos_core::layout::LayoutIndex;

fn bench(c: &mut Criterion) {
    let db = photos_core::Db::open_in_memory().unwrap();
    db.with_conn(|conn| photos_core::synthetic::generate(conn, 250_000, 7)).unwrap();
    let idx = db.with_conn(|c| LayoutIndex::load(c, false)).unwrap();
    c.bench_function("layout_250k_1280px", |b| {
        b.iter(|| idx.compute(1280, 200, 4, Some(photos_core::layout::GroupBy::Month)))
    });
    c.bench_function("index_load_250k", |b| b.iter(|| db.with_conn(|c| LayoutIndex::load(c, false)).unwrap()));
}

criterion_group!(benches, bench);
criterion_main!(benches);
