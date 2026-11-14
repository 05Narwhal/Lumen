use serde::Serialize;
use ts_rs::TS;

/// The one error type that crosses the IPC boundary. `code` is stable and
/// machine-readable; `message` is user-facing; `detail` is for logs and the
/// "Details" toggle.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
}

impl From<photos_core::CoreError> for AppError {
    fn from(e: photos_core::CoreError) -> Self {
        use photos_core::CoreError as C;
        // What the data refused is said in the core's own words, as a sentence.
        if let C::Invalid(why) = &e {
            let mut m: String = why.chars().take(1).flat_map(char::to_uppercase).chain(why.chars().skip(1)).collect();
            m.push('.');
            return Self { code: e.code().into(), message: m, detail: None };
        }
        let message = match &e {
            C::Invalid(_) => unreachable!("handled above"),
            C::NotFound(_) => "The requested item could not be found.",
            C::AlreadyExists(_) => "A library already exists at that location.",
            C::NoLibrary => "No library is open.",
            C::InvalidPath(_) => "That path cannot be used.",
            C::Migration(_) => "The library database could not be upgraded.",
            C::Io(_) => "A file operation failed.",
            C::Db(_) => "The library database reported an error.",
            C::Json(_) => "A settings file is corrupted.",
            C::NewerSchema { .. } => "This library was last opened by a newer Lumen. Update Lumen to open it; nothing in the library was changed.",
            C::SafetyCopy(_) => "Lumen could not make a safety copy before upgrading this library, so it left the library as it was. Free some disk space and open it again.",
        };
        Self { code: e.code().into(), message: message.into(), detail: Some(e.to_string()) }
    }
}

impl From<photos_import::IphoneError> for AppError {
    fn from(e: photos_import::IphoneError) -> Self {
        use photos_import::IphoneError as I;
        let message = match &e {
            I::UsbmuxdUnavailable => "iPhone support needs Apple's device drivers.",
            I::NotPaired => "This iPhone is not paired with this computer.",
            I::TrustRequired => "Unlock the iPhone and tap Trust, then try again.",
            I::Device(_) => "The iPhone could not be read.",
        };
        Self { code: e.code().into(), message: message.into(), detail: Some(e.to_string()) }
    }
}

impl From<photos_plugins::PluginError> for AppError {
    fn from(e: photos_plugins::PluginError) -> Self {
        Self { code: e.code.into(), message: e.message, detail: None }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self { code: "io".into(), message: "A file operation failed.".into(), detail: Some(e.to_string()) }
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        Self {
            code: "tauri".into(),
            message: "The application runtime reported an error.".into(),
            detail: Some(e.to_string()),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_maps_to_stable_code_and_user_message() {
        let e: AppError = photos_core::CoreError::NoLibrary.into();
        assert_eq!(e.code, "no_library");
        assert_eq!(e.message, "No library is open.");
        assert_eq!(serde_json::to_value(&e).unwrap()["code"], "no_library");
    }

    #[test]
    fn a_refusal_is_told_in_the_cores_words() {
        let e: AppError = photos_core::CoreError::Invalid("built-in presets cannot be changed".into()).into();
        assert_eq!((e.code.as_str(), e.message.as_str()), ("invalid", "Built-in presets cannot be changed."));
    }
}
