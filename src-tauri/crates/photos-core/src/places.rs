//! Offline reverse geocoding: latitude and longitude to "Lisbon, Portugal".
//!
//! The dataset is GeoNames `cities15000` (every settlement over 15,000 people, 34k rows)
//! trimmed to name, country, position and population, gzipped to about half a megabyte
//! and compiled into the binary. No network, no API key, no per-photo privacy leak.
//!
//! Lookup buckets the cities into one-degree cells and searches outward from the cell
//! containing the point, so a query touches a few dozen candidates rather than all 34,000
//! — fast enough to run over a whole library on the maintenance lane without an index
//! structure or an extra dependency.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

const CITIES_GZ: &[u8] = include_bytes!("../../../resources/cities.tsv.gz");
const COUNTRIES_GZ: &[u8] = include_bytes!("../../../resources/countries.tsv.gz");

/// Nearest city must be within this to name a place. Beyond it we only claim the country,
/// because "Reykjavík" for a photo 400 km out at sea is worse than saying nothing.
const CITY_RADIUS_KM: f64 = 50.0;
/// Beyond this, not even the country is claimed.
const COUNTRY_RADIUS_KM: f64 = 300.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Place {
    /// `None` when the nearest city is too far to name honestly.
    pub city: Option<String>,
    pub country: Option<String>,
}

impl Place {
    /// "Lisbon, Portugal", "Portugal", or nothing.
    pub fn label(&self) -> Option<String> {
        match (&self.city, &self.country) {
            (Some(c), Some(n)) => Some(format!("{c}, {n}")),
            (Some(c), None) => Some(c.clone()),
            (None, Some(n)) => Some(n.clone()),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone)]
struct City {
    name: String,
    country: String,
    lat: f64,
    lon: f64,
    population: i64,
}

struct Atlas {
    cities: Vec<City>,
    /// (floor(lat), floor(lon)) → indexes into `cities`.
    cells: HashMap<(i32, i32), Vec<u32>>,
    countries: HashMap<String, String>,
}

static ATLAS: OnceLock<Atlas> = OnceLock::new();

fn gunzip(bytes: &[u8]) -> String {
    let mut out = String::new();
    let _ = flate2::read::GzDecoder::new(bytes).read_to_string(&mut out);
    out
}

fn atlas() -> &'static Atlas {
    ATLAS.get_or_init(|| {
        let mut cities = Vec::with_capacity(35_000);
        for line in gunzip(CITIES_GZ).lines() {
            let mut f = line.split('\t');
            let (Some(name), Some(country), Some(lat), Some(lon), Some(pop)) =
                (f.next(), f.next(), f.next(), f.next(), f.next())
            else {
                continue;
            };
            let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else { continue };
            cities.push(City {
                name: name.to_string(),
                country: country.to_string(),
                lat,
                lon,
                population: pop.parse().unwrap_or(0),
            });
        }
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, c) in cities.iter().enumerate() {
            cells.entry((c.lat.floor() as i32, c.lon.floor() as i32)).or_default().push(i as u32);
        }
        let countries = gunzip(COUNTRIES_GZ)
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(cc, name)| (cc.to_string(), name.to_string()))
            .collect();
        Atlas { cities, cells, countries }
    })
}

/// Great-circle distance in kilometres.
pub fn haversine_km(a_lat: f64, a_lon: f64, b_lat: f64, b_lon: f64) -> f64 {
    const R: f64 = 6371.0088;
    let (p1, p2) = (a_lat.to_radians(), b_lat.to_radians());
    let dp = (b_lat - a_lat).to_radians();
    let dl = (b_lon - a_lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}

/// The place a photo was taken, as near as the bundled atlas can say.
pub fn reverse_geocode(lat: f64, lon: f64) -> Option<Place> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let a = atlas();
    let (clat, clon) = (lat.floor() as i32, lon.floor() as i32);
    let mut best: Option<(f64, &City)> = None;
    // One degree of latitude is ~111 km, so a two-cell ring already covers the country
    // radius; widen only while nothing has been found.
    for ring in 0i32..=3 {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                // Only the new ring, not the cells already visited.
                if ring > 0 && dy.abs() != ring && dx.abs() != ring {
                    continue;
                }
                let lon_cell = wrap_lon(clon + dx);
                let Some(ids) = a.cells.get(&(clat + dy, lon_cell)) else { continue };
                for &i in ids {
                    let c = &a.cities[i as usize];
                    let d = haversine_km(lat, lon, c.lat, c.lon);
                    // Ties go to the larger place: people name the city, not the suburb.
                    let better = match best {
                        None => true,
                        Some((bd, bc)) => d < bd - 0.5 || (d < bd + 0.5 && c.population > bc.population),
                    };
                    if better {
                        best = Some((d, c));
                    }
                }
            }
        }
        if best.is_some() && ring >= 1 {
            break;
        }
    }
    let (distance, city) = best?;
    let country = a.countries.get(&city.country).cloned();
    if distance <= CITY_RADIUS_KM {
        Some(Place { city: Some(city.name.clone()), country })
    } else if distance <= COUNTRY_RADIUS_KM {
        Some(Place { city: None, country })
    } else {
        None
    }
}

const LOCALITIES_GZ: &[u8] = include_bytes!("../../../resources/localities.tsv.gz");

/// A photo's locality must be this close: villages in the countryside are a few kilometres
/// apart, and beyond this the nearest one is not where the photo was taken.
pub const LOCALITY_RADIUS_KM: f64 = 5.0;

/// Every populated place of 500 people or more (GeoNames `cities500`, ~236k, Round 4K), so a
/// photo is also named by the village or district it was taken in, and every place in a
/// library can be searched for by name. About 20 MB once loaded, so it is **not** kept: the
/// maintenance pass loads it, names the photos waiting, and drops it.
pub struct Localities {
    places: Vec<City>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Localities {
    pub fn load() -> Localities {
        let mut places = Vec::with_capacity(240_000);
        for line in gunzip(LOCALITIES_GZ).lines() {
            let mut f = line.split('\t');
            let (Some(name), Some(country), Some(lat), Some(lon), Some(pop)) = (f.next(), f.next(), f.next(), f.next(), f.next()) else {
                continue;
            };
            let (Ok(lat), Ok(lon)) = (lat.parse::<f64>(), lon.parse::<f64>()) else { continue };
            places.push(City { name: name.to_string(), country: country.to_string(), lat, lon, population: pop.parse().unwrap_or(0) });
        }
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, c) in places.iter().enumerate() {
            cells.entry(((c.lat * 10.0).floor() as i32, (c.lon * 10.0).floor() as i32)).or_default().push(i as u32);
        }
        Localities { places, cells }
    }

    pub fn len(&self) -> usize {
        self.places.len()
    }

    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    /// "Kropa, Slovenia": the nearest place within `LOCALITY_RADIUS_KM`.
    pub fn nearest(&self, lat: f64, lon: f64) -> Option<String> {
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return None;
        }
        // Tenth-of-a-degree cells (~11 km of latitude); the cells around cover the radius.
        let (clat, clon) = ((lat * 10.0).floor() as i32, (lon * 10.0).floor() as i32);
        let mut best: Option<(f64, &City)> = None;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let Some(ids) = self.cells.get(&(clat + dy, clon + dx)) else { continue };
                for &i in ids {
                    let c = &self.places[i as usize];
                    let d = haversine_km(lat, lon, c.lat, c.lon);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, c));
                    }
                }
            }
        }
        let (_, c) = best.filter(|(d, _)| *d <= LOCALITY_RADIUS_KM)?;
        Some(match atlas().countries.get(&c.country) {
            Some(country) => format!("{}, {country}", c.name),
            None => c.name.clone(),
        })
    }
}

/// Longitude cells wrap at the antimeridian.
fn wrap_lon(cell: i32) -> i32 {
    match cell {
        c if c < -180 => c + 360,
        c if c > 179 => c - 360,
        c => c,
    }
}

/// Cities whose name starts with `prefix`, largest first — the map's place search.
pub fn search(prefix: &str, limit: usize) -> Vec<(String, f64, f64)> {
    let needle = prefix.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let a = atlas();
    let mut hits: Vec<&City> = a.cities.iter().filter(|c| c.name.to_lowercase().starts_with(&needle)).collect();
    hits.sort_by_key(|c| std::cmp::Reverse(c.population));
    hits.truncate(limit);
    hits.iter()
        .map(|c| {
            let label = a.countries.get(&c.country).map(|n| format!("{}, {n}", c.name)).unwrap_or_else(|| c.name.clone());
            (label, c.lat, c.lon)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_well_known_places() {
        let lisbon = reverse_geocode(38.7223, -9.1393).unwrap();
        assert_eq!(lisbon.city.as_deref(), Some("Lisbon"));
        assert_eq!(lisbon.country.as_deref(), Some("Portugal"));
        assert_eq!(lisbon.label().as_deref(), Some("Lisbon, Portugal"));

        let ljubljana = reverse_geocode(46.0569, 14.5058).unwrap();
        assert_eq!(ljubljana.country.as_deref(), Some("Slovenia"));

        let tokyo = reverse_geocode(35.6895, 139.6917).unwrap();
        assert_eq!(tokyo.country.as_deref(), Some("Japan"));
    }

    #[test]
    fn refuses_to_invent_a_place_in_the_middle_of_an_ocean() {
        // Point Nemo: the furthest place on Earth from land.
        assert_eq!(reverse_geocode(-48.876, -123.393), None);
        // Out-of-range coordinates are not a place either.
        assert_eq!(reverse_geocode(91.0, 0.0), None);
        assert_eq!(reverse_geocode(0.0, 181.0), None);
    }

    #[test]
    fn falls_back_to_the_country_when_the_nearest_city_is_far() {
        // Deep in the Sahara: hundreds of kilometres from any town of 15,000 people.
        let p = reverse_geocode(23.5, 12.0);
        if let Some(p) = p {
            assert!(p.city.is_none(), "too far to name a city, got {:?}", p.city);
            assert!(p.country.is_some());
        }
    }

    #[test]
    fn searches_by_prefix_largest_first() {
        let hits = search("lisb", 5);
        assert!(hits.iter().any(|(label, _, _)| label.starts_with("Lisbon")), "{hits:?}");
        assert!(search("", 5).is_empty());
        assert!(search("zzzzzznotaplace", 5).is_empty());
    }

    #[test]
    fn names_the_village_a_photo_was_taken_in() {
        let l = Localities::load();
        // Kropa (807 people) is in no atlas of towns of 15,000; the photo is 100 m from it.
        assert_eq!(l.nearest(46.2895, 14.2048).as_deref(), Some("Kropa, Slovenia"));
        assert_eq!(l.nearest(46.3167, 14.2000).as_deref(), Some("Kamna Gorica, Slovenia"));
        assert_eq!(l.nearest(-48.876, -123.393), None, "nowhere near a village");
        assert_eq!(l.nearest(91.0, 0.0), None);
        assert!(l.len() > 200_000);
    }

    #[test]
    fn measures_distance_between_known_points() {
        // Lisbon to Madrid is about 500 km.
        let d = haversine_km(38.7223, -9.1393, 40.4168, -3.7038);
        assert!((d - 502.0).abs() < 15.0, "{d}");
        assert_eq!(haversine_km(10.0, 20.0, 10.0, 20.0), 0.0);
    }
}
