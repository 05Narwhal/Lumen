//! Exact place names from an online reverse geocoder (OpenStreetMap Nominatim), plan
//! `2026-11-08-exact-place-names`. Pure: the network call lives in the Tauri binary.
//!
//! **Border rule:** a lookup is shared only by photos at the same position rounded to 5
//! decimals (~1 m, below a phone's GPS error), never "nearby" — a 20 m reuse can cross a border.

use serde::Deserialize;

/// The parts of Nominatim's `address` object the names are made from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Address {
    pub city: Option<String>,
    pub town: Option<String>,
    pub municipality: Option<String>,
    pub village: Option<String>,
    pub hamlet: Option<String>,
    pub county: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
}

/// What a lookup gives a photo: the `place_name` and `locality` columns, as the atlas fills them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    /// "Town, Country", "Country", or '' when there is nothing to say.
    pub place_name: String,
    /// "Locality, Country" for the settlement the point is in, '' when none.
    pub locality: String,
    pub country_code: Option<String>,
}

/// The cache key: the position rounded to 5 decimals, `"46.00060,14.02640"`.
pub fn key(lat: f64, lon: f64) -> String {
    fn part(v: f64) -> String {
        let s = format!("{v:.5}");
        // `-0.00000` and `0.00000` are the same place.
        if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { "0.00000".into() } else { s }
    }
    format!("{},{}", part(lat), part(lon))
}

/// Read a `/reverse?format=jsonv2` answer: its `address` (default when the point names nothing,
/// e.g. `{"error":"Unable to geocode"}` at sea) and that object's JSON, kept so new name rules
/// need no network.
pub fn parse_reverse(body: &str) -> serde_json::Result<(Address, String)> {
    let v: serde_json::Value = serde_json::from_str(body)?;
    let raw = v.get("address").cloned().unwrap_or_else(|| serde_json::json!({}));
    let address: Address = serde_json::from_value(raw.clone())?;
    Ok((address, raw.to_string()))
}

fn present(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn with_country(name: Option<&str>, country: Option<&str>) -> String {
    match (name, country) {
        (Some(n), Some(c)) => format!("{n}, {c}"),
        (Some(n), None) => n.to_string(),
        (None, Some(c)) => c.to_string(),
        (None, None) => String::new(),
    }
}

/// Name a photo from an address. Town = city ?? town ?? municipality ?? village ?? county;
/// locality = the settlement the point is in (village ?? hamlet ?? town ?? city), never a
/// suburb, quarter or neighbourhood.
pub fn names_from_address(a: &Address) -> Names {
    let country = present(&a.country);
    let town = present(&a.city)
        .or(present(&a.town))
        .or(present(&a.municipality))
        .or(present(&a.village))
        .or(present(&a.county));
    let locality = present(&a.village).or(present(&a.hamlet)).or(present(&a.town)).or(present(&a.city));
    Names {
        place_name: with_country(town, country),
        locality: locality.map(|l| with_country(Some(l), country)).unwrap_or_default(),
        country_code: present(&a.country_code).map(str::to_lowercase),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Names {
        let path = format!("{}/tests/fixtures/nominatim/{name}.json", env!("CARGO_MANIFEST_DIR"));
        let body = std::fs::read_to_string(&path).unwrap();
        names_from_address(&parse_reverse(&body).unwrap().0)
    }

    #[test]
    fn the_five_measured_points_get_their_exact_names() {
        let cases = [
            ("idrija", "Idrija, Slovenia", "Idrija, Slovenia"),
            ("jelicni-vrh", "Idrija, Slovenia", "Jelični Vrh, Slovenia"),
            ("pristava", "Nova Gorica, Slovenia", "Pristava, Slovenia"),
            ("nova-gorica-station", "Nova Gorica, Slovenia", "Nova Gorica, Slovenia"),
            ("gorizia", "Gorizia, Italy", "Gorizia, Italy"),
        ];
        for (file, place, locality) in cases {
            let n = fixture(file);
            assert_eq!((n.place_name.as_str(), n.locality.as_str()), (place, locality), "{file}");
        }
        assert_eq!(fixture("gorizia").country_code.as_deref(), Some("it"));
        assert_eq!(fixture("idrija").country_code.as_deref(), Some("si"));
    }

    #[test]
    fn the_raw_address_is_kept() {
        let body = std::fs::read_to_string(format!("{}/tests/fixtures/nominatim/idrija.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let (_, raw) = parse_reverse(&body).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["town"], "Idrija");
    }

    #[test]
    fn a_point_with_nothing_to_say_names_nothing() {
        let (a, raw) = parse_reverse(r#"{"error":"Unable to geocode"}"#).unwrap();
        assert_eq!(raw, "{}");
        assert_eq!(names_from_address(&a), Names::default());
    }

    #[test]
    fn only_a_country_names_the_country() {
        let a = Address { country: Some("Iceland".into()), ..Default::default() };
        let n = names_from_address(&a);
        assert_eq!((n.place_name.as_str(), n.locality.as_str()), ("Iceland", ""));
    }

    #[test]
    fn keys_round_to_a_metre_and_never_further() {
        assert_eq!(key(46.0006, 14.0264), "46.00060,14.02640");
        assert_eq!(key(45.955704, 13.637899), "45.95570,13.63790");
        assert_ne!(key(45.95570, 13.63790), key(45.95560, 13.63790));
        assert_eq!(key(-0.000001, 0.0), "0.00000,0.00000");
        assert_eq!(key(-33.8688, 151.2093), "-33.86880,151.20930");
    }
}
