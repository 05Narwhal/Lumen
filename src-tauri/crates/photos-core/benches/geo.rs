//! Map panning at library scale: the roadmap's budget is clusters under 30 ms at 250k.

use criterion::{criterion_group, criterion_main, Criterion};
use photos_core::geo::{map_bounds, map_clusters, Bounds};
use photos_core::scope::LayoutScope;

fn bench(c: &mut Criterion) {
    let db = photos_core::Db::open_in_memory().unwrap();
    db.with_conn(|conn| photos_core::synthetic::generate(conn, 250_000, 7)).unwrap();
    db.with_conn(photos_core::db::analyze).unwrap();
    let scope = LayoutScope::library();

    // A viewport the UI can actually produce: at zoom 8 the map shows about a degree
    // and a half, not a continent. Pairing a wide box with a deep zoom would measure a
    // state the app never reaches.
    let city = Bounds { min_lat: 45.4, max_lat: 46.8, min_lon: 13.8, max_lon: 15.2 };
    c.bench_function("map_clusters_world_z3", |b| {
        b.iter(|| db.with_conn(|c| map_clusters(c, Bounds::world(), 3.0, &scope)).unwrap())
    });
    c.bench_function("map_clusters_city_z8", |b| {
        b.iter(|| db.with_conn(|c| map_clusters(c, city, 8.0, &scope)).unwrap())
    });
    c.bench_function("map_bounds_250k", |b| b.iter(|| db.with_conn(|c| map_bounds(c, &scope)).unwrap()));
}

criterion_group!(benches, bench);
criterion_main!(benches);
