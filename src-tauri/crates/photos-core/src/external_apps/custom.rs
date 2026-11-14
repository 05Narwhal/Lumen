//! The person's own apps (*Choose another app…*): an `Editor` that is always given a copy.

use std::path::Path;

use super::{AppKind, AppSource, CustomApp, InstalledApp, Lookup, Media, Writes};

/// `custom-` and 12 hex characters of the path's blake3, so the same file is the same app.
pub fn custom_id(path: &str) -> String {
    let hex = blake3::hash(path.as_bytes()).to_hex();
    format!("custom-{}", &hex.as_str()[..12])
}

/// The settings entry for a chosen file or `.app` bundle: its name is the file name without the
/// extension.
pub fn custom_app_for(path: &Path) -> CustomApp {
    let text = path.to_string_lossy().into_owned();
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty()).unwrap_or_else(|| text.clone());
    CustomApp { id: custom_id(&text), name, path: text }
}

/// The apps in `externalApps.custom`. One whose file has gone is left out of the list (it stays in
/// the settings, so it returns if the file does).
pub struct Custom {
    pub entries: Vec<CustomApp>,
}

impl AppSource for Custom {
    fn apps(&self, lookup: &dyn Lookup) -> Vec<InstalledApp> {
        self.entries
            .iter()
            .filter(|c| lookup.exists(Path::new(&c.path)))
            .map(|c| InstalledApp {
                id: c.id.clone(),
                name: c.name.clone(),
                kind: AppKind::Editor,
                accepts: vec![Media::Photo, Media::Raw, Media::Video],
                reads_raw: false,
                writes: Writes::InPlace,
                target: c.path.clone().into(),
                exec: None,
                custom: true,
                added_by: None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::fake::FakeLookup;
    use super::super::*;
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn a_custom_app_is_an_editor_that_always_gets_a_copy() {
        let entry = custom_app_for(Path::new("C:/Tools/Fancy Editor.exe"));
        assert_eq!(entry.name, "Fancy Editor");
        assert!(entry.id.starts_with("custom-") && entry.id.len() == "custom-".len() + 12);
        assert_eq!(entry.id, custom_app_for(Path::new("C:/Tools/Fancy Editor.exe")).id);
        assert_ne!(entry.id, custom_app_for(Path::new("C:/Tools/Other.exe")).id);

        let mut l = FakeLookup::default();
        l.files.insert(PathBuf::from("C:/Tools/Fancy Editor.exe"));
        let apps = Custom { entries: vec![entry.clone()] }.apps(&l);
        assert_eq!(apps.len(), 1);
        // Editor + InPlace is what `what_to_send` always answers with a copy.
        assert_eq!((apps[0].kind, apps[0].writes, apps[0].custom), (AppKind::Editor, Writes::InPlace, true));
        // A file that went away leaves the list but not the settings.
        assert!(Custom { entries: vec![entry] }.apps(&FakeLookup::default()).is_empty());
    }

    #[test]
    fn custom_apps_round_trip_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let mut s = Settings::default();
        let entry = custom_app_for(Path::new("C:/Tools/Fancy Editor.exe"));
        s.external_apps.custom.push(entry.clone());
        s.external_apps.defaults.photo = Some(entry.id.clone());
        s.external_apps.send.insert("photoshop".into(), SendMode::Rendered);
        s.save(&file).unwrap();

        let back = Settings::load(&file).unwrap();
        assert_eq!(back.external_apps.custom, vec![entry.clone()]);
        assert_eq!(back.external_apps.defaults.photo.as_deref(), Some(entry.id.as_str()));
        assert_eq!(back.external_apps.send.get("photoshop"), Some(&SendMode::Rendered));
        // The keys are the ones the plan names.
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(json["externalApps"]["custom"].is_array());
        assert_eq!(json["externalApps"]["defaults"]["photo"], entry.id);
        assert_eq!(json["externalApps"]["send"]["photoshop"], "rendered");
        // Older settings files load with nothing set.
        let old: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(old.external_apps.custom.is_empty() && old.external_apps.send.is_empty());
    }
}
