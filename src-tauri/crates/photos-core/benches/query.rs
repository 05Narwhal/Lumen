//! Search at library scale. The budget from the architecture spec §15 is that a query
//! over 250k items answers within one frame; these numbers are the check.

use criterion::{criterion_group, criterion_main, Criterion};
use photos_core::layout::{count_scoped, LayoutIndex};
use photos_core::query::AssetQuery;
use photos_core::scope::LayoutScope;

fn bench(c: &mut Criterion) {
    let db = photos_core::Db::open_in_memory().unwrap();
    db.with_conn(|conn| photos_core::synthetic::generate(conn, 250_000, 7)).unwrap();
    let indexed = db.with_conn(photos_core::fts::rebuild).unwrap();
    assert_eq!(indexed, 250_000);
    // The planner needs statistics over real data; the app runs this on the maintenance lane.
    db.with_conn(photos_core::db::analyze).unwrap();

    let text = LayoutScope {
        query: Some(AssetQuery { text: Some("img 004".into()), ..Default::default() }),
        ..Default::default()
    };
    let structured = LayoutScope {
        query: Some(AssetQuery { min_rating: Some(3), media_type: Some(0), has_gps: Some(true), ..Default::default() }),
        ..Default::default()
    };

    c.bench_function("count_text_250k", |b| b.iter(|| db.with_conn(|c| count_scoped(c, &text)).unwrap()));
    c.bench_function("count_structured_250k", |b| b.iter(|| db.with_conn(|c| count_scoped(c, &structured)).unwrap()));
    c.bench_function("index_load_text_250k", |b| {
        b.iter(|| db.with_conn(|c| LayoutIndex::load_scoped(c, &text, false)).unwrap())
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
