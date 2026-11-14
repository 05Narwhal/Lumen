//! Round 4C: People's grouping replayed on public pet identities (never Julian's photos), to
//! choose the pet thresholds. The input is `scripts/ml/eval_pets.py export-bin`'s file: crops
//! of individual animals, each with the animal it shows, embedded and stored as Lumen stores them.
//!
//! ```sh
//! LUMEN_PET_EMBEDDINGS=<identity_dog.dinov2.bin> cargo test --release -p photos-core --test pets_grouping_public -- --ignored --nocapture
//! ```
//!
//! Two situations, each on random households of 2–4 animals and on the most alike pairs:
//! - **first look**: nothing named; groups form from the leftovers (Chinese Whispers). Pairwise
//!   precision and recall of the groups formed.
//! - **known pets**: half of each animal's photos confirmed, the rest new. How many are placed,
//!   how many of those on the right animal, and how many are left to ask about.

use std::collections::HashMap;

use photos_core::embedding::Vector;
use photos_core::people::{GroupingRows, ReclusterInput, Thresholds};

fn load() -> (Vec<i32>, Vec<Vector>) {
    let path = std::env::var("LUMEN_PET_EMBEDDINGS").expect("LUMEN_PET_EMBEDDINGS");
    let b = std::fs::read(path).unwrap();
    let n = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
    let d = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let (mut who, mut vecs) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut at = 8;
    for _ in 0..n {
        who.push(i32::from_le_bytes(b[at..at + 4].try_into().unwrap()));
        vecs.push(b[at + 4..at + 4 + d].iter().map(|&x| x as i8).collect());
        at += 4 + d;
    }
    (who, vecs)
}

fn cos(a: &[i8], b: &[i8]) -> f32 {
    let (mut d, mut na, mut nb) = (0i32, 0i32, 0i32);
    for (x, y) in a.iter().zip(b) {
        d += *x as i32 * *y as i32;
        na += *x as i32 * *x as i32;
        nb += *y as i32 * *y as i32;
    }
    d as f32 / ((na as f32).sqrt() * (nb as f32).sqrt()).max(1.0)
}

/// A tiny deterministic generator, so a run can be repeated.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    pairs_same_group: u64,
    pairs_right: u64,
    pairs_truth: u64,
    placed: u64,
    placed_right: u64,
    open: u64,
}

/// First look: every crop of the household open; what new groups form.
fn first_look(t: &Thresholds, crops: &[(i32, &Vector)], tally: &mut Tally) {
    let rows = GroupingRows {
        members: crops.iter().enumerate().map(|(i, (_, v))| (i as i64, i as i64, None, false, (*v).clone(), 1.0, false)).collect(),
        ..Default::default()
    };
    let plan = ReclusterInput::from_rows(rows).plan(t);
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    for (g, (_, members)) in plan.new_groups().enumerate() {
        for (m, _) in members {
            group_of.insert(*m as usize, g);
        }
    }
    for i in 0..crops.len() {
        for j in i + 1..crops.len() {
            let same = crops[i].0 == crops[j].0;
            tally.pairs_truth += same as u64;
            if let (Some(a), Some(b)) = (group_of.get(&i), group_of.get(&j)) {
                if a == b {
                    tally.pairs_same_group += 1;
                    tally.pairs_right += same as u64;
                }
            }
        }
    }
}

/// Known pets: the first half of each animal's crops confirmed on its own group, the rest new.
fn known_pets(t: &Thresholds, crops: &[(i32, &Vector)], tally: &mut Tally) {
    let mut seen: HashMap<i32, usize> = HashMap::new();
    let mut count: HashMap<i32, usize> = HashMap::new();
    for (w, _) in crops {
        *count.entry(*w).or_default() += 1;
    }
    let mut rows = GroupingRows::default();
    for (i, (w, v)) in crops.iter().enumerate() {
        let k = seen.entry(*w).or_default();
        let confirmed = *k < count[w] / 2;
        *k += 1;
        let group = confirmed.then_some(*w as i64 + 1);
        rows.members.push((i as i64, i as i64, group, confirmed, (*v).clone(), 1.0, false));
        if confirmed {
            rows.evidence.push((*w as i64 + 1, (*v).clone(), 1.0, true));
        }
    }
    rows.evidence.sort_by_key(|e| e.0);
    let open: Vec<usize> = rows.members.iter().enumerate().filter(|(_, m)| m.2.is_none()).map(|(i, _)| i).collect();
    let plan = ReclusterInput::from_rows(rows).plan(t);
    let placed: HashMap<i64, i64> = plan.places().iter().map(|(m, g, _)| (*m, *g)).collect();
    for i in open {
        match placed.get(&(i as i64)) {
            Some(g) => {
                tally.placed += 1;
                tally.placed_right += (*g == crops[i].0 as i64 + 1) as u64;
            }
            None => tally.open += 1,
        }
    }
}

#[test]
#[ignore = "needs public pet embeddings (scripts/ml/eval_pets.py)"]
fn pet_thresholds_on_public_identities() {
    let (who, vecs) = load();
    let mut by: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, w) in who.iter().enumerate() {
        by.entry(*w).or_default().push(i);
    }
    let animals: Vec<i32> = {
        let mut a: Vec<i32> = by.iter().filter(|(_, v)| v.len() >= 4).map(|(k, _)| *k).collect();
        a.sort();
        a
    };
    // The most alike pairs of animals (by the mean of their crops): the "two black cats".
    let centroid = |w: i32| -> Vec<f32> {
        let idx = &by[&w];
        let d = vecs[0].len();
        (0..d).map(|k| idx.iter().map(|&i| vecs[i][k] as f32).sum::<f32>() / idx.len() as f32).collect()
    };
    let cents: Vec<Vec<i8>> = animals.iter().map(|&w| centroid(w).iter().map(|x| x.round().clamp(-127.0, 127.0) as i8).collect()).collect();
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for i in 0..animals.len() {
        for j in i + 1..animals.len() {
            pairs.push((cos(&cents[i], &cents[j]), i, j));
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    // DogFaceNet_large is not curated: the same dog sometimes has two folders, and its photos
    // are then near-identical across them. Such a pair is one animal, not two look-alikes.
    let shares_a_photo = |a: i32, b: i32| by[&a].iter().any(|&i| by[&b].iter().any(|&j| cos(&vecs[i], &vecs[j]) >= 0.97));
    let dropped = pairs.iter().take_while(|p| p.0 > 0.9).filter(|p| shares_a_photo(animals[p.1], animals[p.2])).count();
    pairs.retain(|p| !shares_a_photo(animals[p.1], animals[p.2]));
    println!("{dropped} of the closest pairs share a near-identical photo (one dog, two folders) and are left out");
    let alike: Vec<[i32; 2]> = pairs.iter().take(40).map(|p| [animals[p.1], animals[p.2]]).collect();
    let mut rng = Rng(0x5eed);
    let households: Vec<Vec<i32>> = (0..150)
        .map(|_| {
            let k = 2 + rng.pick(3);
            let mut h: Vec<i32> = Vec::new();
            while h.len() < k {
                let a = animals[rng.pick(animals.len())];
                if !h.contains(&a) {
                    h.push(a);
                }
            }
            h
        })
        .collect();
    let crops_of = |h: &[i32]| -> Vec<(i32, &Vector)> { h.iter().flat_map(|w| by[w].iter().map(|&i| (*w, &vecs[i]))).collect() };

    println!("{} animals with 4+ crops; {} random households, {} look-alike pairs (mean cosine of the closest {:.3})", animals.len(), households.len(), alike.len(), pairs[0].0);
    println!("join | margin | cluster | first look: precision, recall (random) | (alike) | known: placed, right (random) | placed, right, asked (alike)");
    for join in [0.45f32, 0.5, 0.55, 0.6, 0.65] {
        for margin in [0.05f32, 0.1] {
            for cluster in [join, join + 0.05] {
                let t = Thresholds { join, margin, suggest: join - 0.1, cluster, place_quality: 0.0 };
                let (mut fr, mut fa, mut kr, mut ka) = (Tally::default(), Tally::default(), Tally::default(), Tally::default());
                for h in &households {
                    let c = crops_of(h);
                    first_look(&t, &c, &mut fr);
                    known_pets(&t, &c, &mut kr);
                }
                for p in &alike {
                    let c = crops_of(p);
                    first_look(&t, &c, &mut fa);
                    known_pets(&t, &c, &mut ka);
                }
                let pct = |a: u64, b: u64| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
                println!(
                    "{join:.2} | {margin:.2} | {cluster:.2} | {:.1}%, {:.1}% | {:.1}%, {:.1}% | {:.1}%, {:.1}% | {:.1}%, {:.1}%, {:.1}%",
                    pct(fr.pairs_right, fr.pairs_same_group),
                    pct(fr.pairs_right, fr.pairs_truth),
                    pct(fa.pairs_right, fa.pairs_same_group),
                    pct(fa.pairs_right, fa.pairs_truth),
                    pct(kr.placed, kr.placed + kr.open),
                    pct(kr.placed_right, kr.placed),
                    pct(ka.placed, ka.placed + ka.open),
                    pct(ka.placed_right, ka.placed),
                    pct(ka.open, ka.placed + ka.open),
                );
            }
        }
    }
}
