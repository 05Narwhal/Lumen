//! `warm_up {amount?, photo?}`: moves the Basic temperature of the photo (or of every photo in
//! Develop's filmstrip) by `amount` (default 8, -100 to 100) through a draft. Lumen clamps every
//! value to its slider and records one undo step per photo.
//!
//! Only the pure helpers build natively (`cargo test`); the plugin itself is wasm32-only.

use serde_json::Value;

#[cfg(target_arch = "wasm32")]
mod plugin {
    use lumen_plugin::{drafts, log, photos, Tool, Tools};
    use serde_json::{json, Value};

    use super::{amount_of, current_temperature, target};

    struct Warmth;

    impl Tools for Warmth {
        fn tools() -> Vec<Tool> {
            let schema = json!({
                "type": "object",
                "properties": {
                    "amount": { "type": "number", "minimum": -100, "maximum": 100, "description": "How far to move the temperature (default 8). Negative is cooler." },
                    "photo": { "type": "integer", "description": "One photo's id from the filmstrip. Leave out for every photo in the filmstrip." }
                }
            });
            vec![Tool { name: "warm_up".into(), input_schema: schema.to_string() }]
        }

        fn call(name: String, args_json: String) -> Result<String, String> {
            if name != "warm_up" {
                return Err(format!("no tool {name}"));
            }
            let args: Value = if args_json.trim().is_empty() { json!({}) } else { serde_json::from_str(&args_json).map_err(|e| format!("the arguments aren't JSON: {e}"))? };
            let amount = amount_of(&args)?;
            let scope = photos::scope();
            let ids: Vec<i64> = match args.get("photo").and_then(Value::as_i64) {
                Some(id) if scope.contains(&id) => vec![id],
                Some(id) => return Err(format!("photo {id} isn't in Develop's filmstrip")),
                None => scope,
            };
            if ids.is_empty() {
                return Err("Develop's filmstrip is empty".into());
            }
            let mut changed = Vec::new();
            for id in ids {
                let temperature = target(current_temperature(&photos::recipe(id)?), amount);
                let patch = json!({ "develop": { "basic": { "temperature": temperature } } });
                drafts::patch(id, &patch.to_string(), "warm up")?;
                changed.push(json!({ "id": id, "temperature": temperature }));
            }
            log(&format!("warmed {} photos by {amount}", changed.len()));
            Ok(json!({ "changed": changed }).to_string())
        }
    }

    lumen_plugin::export_tools!(Warmth);
}

pub const DEFAULT_AMOUNT: f64 = 8.0;

/// The photo's current temperature from its recipe JSON (`develop.basic.temperature`; 0 when the
/// recipe leaves it out).
pub fn current_temperature(recipe: &str) -> f64 {
    serde_json::from_str::<Value>(recipe).ok().and_then(|v| v["develop"]["basic"]["temperature"].as_f64()).unwrap_or(0.0)
}

pub fn target(current: f64, amount: f64) -> f64 {
    (current + amount).clamp(-100.0, 100.0)
}

pub fn amount_of(args: &Value) -> Result<f64, String> {
    match args.get("amount") {
        None | Some(Value::Null) => Ok(DEFAULT_AMOUNT),
        Some(v) => v.as_f64().filter(|a| a.is_finite() && (-100.0..=100.0).contains(a)).ok_or_else(|| "amount is a number from -100 to 100".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_clamps_the_temperature() {
        assert_eq!(current_temperature(r#"{"develop":{"basic":{"temperature":12.5}}}"#), 12.5);
        assert_eq!(current_temperature(r#"{"develop":{}}"#), 0.0);
        assert_eq!(current_temperature("nope"), 0.0);
        assert_eq!(target(95.0, 8.0), 100.0);
        assert_eq!(target(-95.0, -8.0), -100.0);
    }

    #[test]
    fn amount_is_checked() {
        assert_eq!(amount_of(&json!({})), Ok(DEFAULT_AMOUNT));
        assert_eq!(amount_of(&json!({"amount": -20})), Ok(-20.0));
        assert!(amount_of(&json!({"amount": 500})).is_err());
        assert!(amount_of(&json!({"amount": "x"})).is_err());
    }
}
