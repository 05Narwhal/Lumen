//! Lumen Setup: the decisions (phase 3) live in these modules so tests reach them without a window.
pub mod cleanup;
pub mod commands;
pub mod folder;
pub mod install;
pub mod libraries;
pub mod lumen;
pub mod models;
pub mod options;
pub mod registry;
pub mod payload;
pub mod removal;
pub mod shortcuts;
pub mod uninstall;
pub mod version;

/// `--uninstall` launches are routed before any window (`uninstall::route`); the stock
/// installer's pass-through exits with its uninstaller's code.
pub fn main_entry() {
    if let Some(code) = uninstall::route() {
        std::process::exit(code);
    }
    run();
}

/// Where Setup's own WebView2 keeps its profile: the temp folder, so a finished Setup leaves nothing
/// in `%LOCALAPPDATA%` (Tauri's default is `%LOCALAPPDATA%\<identifier>`, which is Lumen's neighbour).
/// One fixed name, so repeated runs reuse it instead of piling up; `cleanup` deletes it at exit.
pub fn webview_data_dir(temp: &std::path::Path) -> std::path::PathBuf {
    temp.join(cleanup::PROFILE_NAME)
}

pub fn run() {
    tauri::Builder::default()
        .manage(models::ModelsJob::new())
        .manage(commands::LumenWait::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_install_state,
            commands::check_folder,
            commands::choose_folder,
            commands::install,
            commands::skip_models,
            commands::open_lumen,
            commands::cancel_wait,
            commands::uninstall,
            commands::library_details,
            commands::check_recycle_bin
        ])
        // The window is made here, not from tauri.conf.json (`create: false`), to give it that profile.
        .setup(|app| {
            let config = app.config().app.windows.first().cloned().ok_or("tauri.conf.json has no window")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.data_directory(webview_data_dir(&std::env::temp_dir())).build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Lumen Setup")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                cleanup::start_after_exit(&webview_data_dir(&std::env::temp_dir()));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_webview_profile_lives_in_the_temp_folder_under_one_name() {
        let dir = webview_data_dir(std::path::Path::new("C:/Users/A/AppData/Local/Temp"));
        assert_eq!(dir, std::path::Path::new("C:/Users/A/AppData/Local/Temp").join("Lumen Setup WebView"));
    }
}
