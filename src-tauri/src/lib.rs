pub mod ai_task;
pub mod backup;
pub mod library_check;
pub mod repair; // Round 21 lane B
pub mod library_health; // Round 22 lane B
pub mod temp_files; // Round 22 lane B
pub mod external_apps; // Round 22 lane C
pub mod external_edits; // Round 22 lane C
pub mod beat_analysis;
mod commands;
pub mod embeddings;
pub mod faces;
pub mod facet;
pub mod face_radar;
mod clipboard;
mod eject;
mod connected;
mod device_browser;
pub mod develop_auto;
pub mod discord_presence;
pub mod develop_retouch;
mod develop_preview;
mod edit_preview;
mod macdisks;
mod mcp;
pub mod plugins; // Round 23
pub mod derivatives;
pub mod denoise;
mod error;
mod events;
mod watchdog;
mod jobs_ticker;
mod locale;
mod logging;
pub mod maintenance;
pub mod memory_prep; // Round 21 lane C
pub mod model_bench;
pub mod model_catalogue;
pub mod models;
pub mod search;
pub mod music;
pub mod track_profiles;
pub mod video_highlights;
pub mod video_prep;
pub mod ocr;
pub mod pets;
pub mod places_online;
// Round 24 lane W
pub mod originals_gps;
mod protocol;
pub mod slideshows;
mod state;
mod iphones;
mod volumes;
mod restart;
mod crash_marker; // Round 24 lane B
mod space;
mod updater;
mod vault;
mod window;
mod window_edge;
#[cfg(test)]
mod db_speed;

use state::AppState;

/// `Lumen --mcp`: the stdio bridge to the running app's MCP server (Round 16); the exit code.
pub fn mcp_stdio() -> i32 {
    mcp::stdio::main()
}

/// `Lumen --check-library <library>` (Round 17, the release check): a copy of the library's
/// database in a temp folder is opened and migrated by this build, and the result printed as JSON.
/// The library is only read; the copy is removed. The exit code: 0 when it opened.
pub fn check_library(root: &std::path::Path) -> i32 {
    let scratch = std::env::temp_dir().join(format!("lumen-check-{}", std::process::id()));
    match photos_core::library::check_upgrade(root, &scratch) {
        Ok(r) => {
            println!("{}", serde_json::json!({ "ok": true, "from": r.from, "to": r.to, "version": env!("CARGO_PKG_VERSION") }));
            0
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "ok": false, "why": e.to_string(), "version": env!("CARGO_PKG_VERSION") }));
            1
        }
    }
}

/// `Lumen --quit` (Round 18 A2) when no other Lumen from this same program runs: nothing to close,
/// so it ends before opening a library. Otherwise the single-instance check hands it over.
pub fn quit_finds_nothing() -> bool {
    let me = std::process::id();
    match std::env::current_exe() {
        Ok(exe) => !updater::other_lumen_open(me, &exe.to_string_lossy(), &updater::processes()),
        Err(_) => false,
    }
}

pub fn run() {
    logging::init();
    let launch = window::launch_flags(std::env::args());
    restart::wait_for_previous();
    // Every rayon job (resizing, develop, clustering) is background work: one core is left
    // to the UI, and the rest run below normal priority so the webview always wins.
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(cores.saturating_sub(1).max(1))
        .thread_name(|i| format!("lumen-rayon-{i}"))
        .start_handler(|_| photos_jobs::background_priority())
        .build_global();

    // The RAW library's installed path, before anything can open a RAW file.
    let context = tauri::generate_context!();
    if let Ok(dir) = tauri::utils::platform::resource_dir(context.package_info(), &tauri::Env::default()) {
        photos_media::raw_lib::configure(dir.join("raw").join(photos_media::raw_lib::FILE_NAME));
    }

    let builder = tauri::Builder::default()
        // First, so a second launch hands its flags over and ends before anything else starts
        // (Round 18 A2: Setup's --quit; a plain second launch focuses this window). The app
        // state is built in `setup`, after this plugin: bootstrapping opens the library and
        // settles "interrupted" imports, which a second launch must never do to the running
        // Lumen's import (it deleted the copies that import had just placed).
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            plugins::state::forward_package(app, &args); // Round 23 C1: a double-clicked .lumenplugin
            window::on_second_launch(app, &args)
        }))
        .manage(updater::Updates::default())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("photos", protocol::handle)
        .on_window_event(window::on_window_event)
        .setup(move |app| {
            if launch.quit {
                // The Lumen Setup wanted to close ended between `quit_finds_nothing` and here.
                window::quit_for_setup();
                app.handle().exit(0);
                return Ok(());
            }
            // Round 24 lane B: after the single-instance check, before anything that can crash.
            if let Ok(paths) = photos_core::paths::AppPaths::resolve() {
                crash_marker::start(&paths.app_data);
            }
            // Security track: a staged security fix installs now, before the library is opened.
            updater::install_at_launch(app.handle());
            tauri::Manager::manage(app, AppState::bootstrap().expect("failed to bootstrap Lumen app state"));
            // Round 23: plugins.json and the plugin folders' manifests only; never fails boot.
            let plugins_app_dir = photos_core::paths::AppPaths::resolve()
                .map(|p| p.app_data)
                .unwrap_or_else(|_| photos_core::paths::AppPaths::app_data_under(std::path::Path::new(".")));
            let loaded = std::sync::Arc::new(plugins::Plugins::load(&plugins_app_dir));
            loaded.attach(app.handle().clone()); // Round 23 C1: for events from a plugin that stops on its own
            // Round 23 A4: the MCP server reads plugins' tools again after any change.
            plugins::mcp_tools::follow_changes(&loaded, tauri::Manager::state::<AppState>(app).mcp.shared.clone());
            // A `.lumenplugin` this launch was opened with waits for the page to ask for it.
            if let Some(path) = plugins::state::package_arg(&std::env::args().collect::<Vec<_>>()) {
                loaded.set_pending(path);
            }
            // Round 24 lane C: the plugin store's index, downloads and background updates.
            let store = std::sync::Arc::new(plugins::store::StoreService::new(&plugins_app_dir));
            plugins::store::spawn_background(app.handle().clone(), store.clone(), loaded.clone());
            tauri::Manager::manage(app, store);
            tauri::Manager::manage(app, loaded);
            // Round 26 B1: installed .lumentheme themes, apart from plugins.
            tauri::Manager::manage(app, std::sync::Arc::new(plugins::themes::Themes::load(&plugins_app_dir)));
            // Round 23 B2: detection reads the plugins' apps.toml.
            external_apps::use_plugins(tauri::Manager::state::<std::sync::Arc<plugins::Plugins>>(app).inner().clone());
            // Round 22 lane A: an oversized image's decode budget, read by Develop's base too.
            let _ = derivatives::oversize_budget();
            window::apply_saved_state(app, launch.minimized)?;
            window_edge::install(app);
            jobs_ticker::start(app.handle().clone());
            watchdog::start(app.handle().clone());
            volumes::start(app.handle().clone());
            iphones::start(app.handle().clone());
            macdisks::start(app.handle().clone());
            maintenance::start(app.handle().clone());
            backup::start_schedule(app.handle().clone());
            mcp::apply(app.handle());
            updater::start(app.handle().clone());
            discord_presence::start(app.handle().clone());
            Ok(())
        });

    let builder = builder.invoke_handler(tauri::generate_handler![
        commands::app::get_app_info,
        commands::app::get_free_space,
        commands::storage::storage_overview,
        commands::storage::get_temp_usage, // Round 22 lane B
        commands::storage::clear_temp,
        commands::app::ping,
        commands::app::paint_window_edge,
        commands::app::ui_heartbeat,
        commands::app::confirm_close,
        commands::app::hold_close,
        commands::app::restart_lumen,
        commands::app::resume_info, // Round 24 lane B
        commands::app::note_route,
        commands::updates::update_status,
        commands::updates::check_for_updates,
        commands::updates::download_update,
        commands::updates::install_update_now,
        commands::updates::take_update_receipt,
        commands::settings::get_settings,
        commands::settings::update_settings,
        commands::mcp::mcp_set_scope,
        commands::mcp::mcp_set_mounted,
        commands::mcp::mcp_transform,
        commands::mcp::mcp_reply,
        commands::mcp::mcp_status,
        commands::mcp::mcp_regenerate_token,
        commands::mcp::mcp_connect,
        commands::discord::discord_set_screen,
        commands::discord::discord_info,
        commands::settings::get_date_order,
        commands::settings::system_date_order,
        commands::lightroom::import_lightroom_presets,
        commands::lightroom::camera_raw_preset_folder,
        commands::lightroom::find_lightroom_catalogs,
        commands::lightroom::start_lightroom_import,
        commands::library::list_libraries,
        commands::library::get_current_library,
        commands::library::library_exists_at,
        commands::library::list_unreadable,
        commands::library::trashed_unreadable_ids,
        commands::library::unreadable_source_folders,
        commands::library::count_unreadable,
        commands::library::retry_unreadable,
        commands::library::create_library,
        commands::library::open_library,
        commands::library::forget_library,
        commands::library::locate_library,
        commands::library::get_missing_library,
        commands::library::get_library_problem,
        commands::import::start_folder_import,
        commands::import::start_files_import,
        commands::import::running_import,
        commands::import::cancel_import,
        commands::import::retry_import_failures,
        commands::import::list_import_batches,
        commands::import::get_import_failures,
        commands::import::get_import_skips,
        commands::catalog::get_layout,
        commands::catalog::count_scope,
        commands::search::search_overview,
        commands::search::suggest_tags,
        commands::search::read_search_sentence,
        commands::catalog::scope_ids,
        commands::catalog::layout_range_ids,
        commands::catalog::list_collections,
        commands::catalog::rebuild_search_index,
        commands::catalog::get_layout_rows,
        commands::catalog::find_layout_row,
        commands::catalog::get_scope_neighbors,
        commands::catalog::get_section_neighbors,
        commands::catalog::layout_ids,
        commands::catalog::get_asset,
        commands::catalog::get_asset_neighbors,
        commands::catalog::get_library_stats,
        commands::catalog::home_summary,
        commands::catalog::get_recent_asset_ids,
        commands::catalog::prefetch_previews,
        commands::ocr::asset_text,
        commands::facet::ai_masks_list,
        commands::facet::ai_masks_detect,
        commands::ocr::prefetch_text,
        commands::catalog::set_favorite,
        commands::catalog::set_rating,
        commands::catalog::set_color_label,
        commands::catalog::set_flag,
        commands::catalog::get_favorite_ids,
        commands::catalog::trash_assets,
        commands::catalog::restore_assets,
        commands::catalog::delete_now,
        commands::catalog::empty_trash,
        commands::albums::list_albums,
        commands::albums::get_album,
        commands::albums::create_album,
        commands::albums::create_smart_album,
        commands::albums::update_smart_album,
        commands::albums::rename_album,
        commands::albums::set_album_hides,
        commands::import::plan_import,
        commands::vault::vault_status,
        commands::vault::vault_set_password,
        commands::vault::vault_unlock,
        commands::vault::vault_lock,
        commands::vault::vault_reset_password,
        commands::vault::move_to_vault,
        commands::vault::move_from_vault,
        commands::albums::delete_album,
        commands::albums::duplicate_album,
        commands::albums::add_to_album,
        commands::albums::remove_from_album,
        commands::albums::move_in_album,
        commands::albums::set_album_cover,
        commands::albums::set_album_cover_style,
        commands::albums::shuffle_album_cover,
        commands::albums::set_collection_cover_style,
        commands::pets::list_pets,
        commands::pets::list_removed_pets,
        commands::pets::name_pet,
        commands::pets::merge_pets,
        commands::pets::undo_merge_pets,
        commands::pets::split_pet,
        commands::pets::move_pet_photos,
        commands::pets::restore_pet_places,
        commands::pets::tag_pet,
        commands::pets::remove_pet,
        commands::pets::restore_removed_pet,
        commands::pets::not_a_pet,
        commands::pets::undo_not_a_pet,
        commands::pets::set_pet_cover,
        commands::pets::pet_suggestions,
        commands::pets::answer_pet_suggestion,
        commands::pets::lift_pet_refusal,
        commands::pets::asset_pets,
        commands::pets::untag_pet_detection,
        commands::pets::reject_pet_detection,
        commands::pets::undo_reject_pet_detection,
        commands::pets::create_pet_with_detection,
        commands::pets::undo_create_pet,
        commands::backup::get_backup,
        commands::backup::set_backup_destination,
        commands::backup::set_backup_schedule,
        commands::backup::start_backup,
        commands::backup::cancel_backup,
        commands::library_check::get_check_status,
        commands::library_check::start_check_folder,
        commands::library_check::start_check_library,
        commands::library_check::cancel_check,
        commands::library_check::get_check_items,
        commands::library_check::get_check_problems,
        commands::library_check::save_check_report,
        commands::albums::list_album_collections,
        commands::albums::create_album_collection,
        commands::albums::rename_album_collection,
        commands::albums::move_album_to_collection,
        commands::albums::move_album_collection,
        commands::albums::delete_album_collection,
        commands::albums::restore_album_collection,
        commands::catalog::reveal_asset,
        commands::catalog::copy_image_to_clipboard,
        commands::catalog::copy_assets_to,
        commands::devices::list_volumes,
        commands::devices::get_connected_device,
        commands::memories::list_memories,
        commands::memories::get_memory,
        commands::memories::memory_curated_ids,
        commands::memories::set_memory_hidden,
        commands::memories::regenerate_memories,
        commands::music::music_search,
        commands::music::music_download,
        commands::music::list_library_music,
        commands::music::delete_library_music,
        commands::slideshows::list_slideshows,
        commands::slideshows::get_slideshow,
        commands::slideshows::create_slideshow,
        commands::slideshows::create_slideshow_from_memory,
        commands::slideshows::rebuild_slideshow,
        commands::slideshows::sync_slideshow_to_music,
        commands::slideshows::music_window,
        commands::slideshows::slideshow_clips,
        commands::slideshows::video_best_part,
        commands::slideshows::memory_film,
        commands::slideshows::save_memory_film,
        commands::slideshows::export_memory_film,
        commands::ml::get_ml_status,
        commands::people::list_people,
        commands::people::list_person_groups,
        commands::people::create_person_group,
        commands::people::rename_person_group,
        commands::people::delete_person_group,
        commands::people::set_person_group_members,
        commands::people::name_person,
        commands::people::tag_face,
        commands::people::create_person_with_face,
        commands::people::undo_create_person,
        commands::people::add_face_box,
        commands::people::untag_face,
        commands::people::undo_untag_face,
        commands::people::reject_face,
        commands::people::undo_reject_face,
        commands::people::merge_people,
        commands::people::restore_person,
        commands::people::move_face,
        commands::people::restore_face,
        commands::people::person_suggestions,
        commands::people::answer_suggestion,
        commands::people::undo_suggestion,
        commands::people::move_person_photos,
        commands::people::restore_faces,
        face_radar::set_face_radar_view,
        commands::people::list_removed_people,
        commands::people::list_rejected_faces,
        commands::people::remove_person,
        commands::people::restore_removed_person,
        commands::people::reject_person,
        commands::people::undo_reject_person,
        commands::people::set_person_cover,
        commands::people::faces_for_asset,
        commands::ml::fetch_models,
        commands::slideshows::save_slideshow,
        commands::slideshows::delete_slideshow,
        commands::slideshows::export_slideshow,
        commands::edits::get_edit,
        commands::edits::save_edit,
        commands::edits::rotate_assets,
        commands::edits::revert_edit,
        commands::edits::list_edit_versions,
        commands::edits::render_edit_preview,
        commands::edits::export_assets,
        commands::edits::export_summary,
        commands::develop::develop_preview,
        commands::develop::develop_auto,
        commands::develop::develop_retouch,
        commands::develop::experimental_catalog,
        commands::develop::retest_models,
        commands::denoise::denoise_status,
        commands::denoise::start_denoise,
        commands::denoise::cancel_denoise,
        commands::ai::ai_cancel,
        commands::develop::develop_region,
        commands::develop::develop_before,
        commands::duplicates::list_duplicate_sets,
        commands::duplicates::count_duplicate_sets,
        commands::duplicates::resolve_duplicates,
        commands::duplicates::undo_resolve_duplicates,
        commands::duplicates::dismiss_duplicates,
        commands::duplicates::undo_dismiss_duplicates,
        commands::develop::develop_release_full,
        commands::develop::develop_base_info,
        commands::develop::list_presets,
        commands::develop::save_preset,
        commands::develop::rename_preset,
        commands::develop::delete_preset,
        commands::develop::duplicate_plugin_preset,
        commands::develop::list_history,
        commands::develop::push_history,
        commands::develop::truncate_history,
        commands::develop::clear_history,
        commands::develop::save_snapshot,
        commands::develop::rename_snapshot,
        commands::develop::delete_snapshot,
        commands::develop::paste_develop,
        commands::develop::restore_recipes,
        commands::develop::get_develop_draft,
        commands::develop::save_develop_draft,
        commands::develop::restore_develop_drafts,
        commands::develop::strip_page,
        commands::develop::strip_ids,
        commands::apple::list_mac_disks,
        commands::apple::find_apple_photos_libraries,
        commands::apple::list_mac_folder,
        commands::apple::start_mac_folder_import,
        commands::apple::get_photos_library,
        commands::apple::start_photos_library_import,
        commands::devices::start_volume_import,
        commands::devices::forget_volume,
        commands::devices::eject_volume,
        commands::devices::list_iphones,
        commands::devices::start_iphone_import,
        commands::devices::list_devices,
        commands::devices::rename_device,
        commands::devices::set_device_hidden,
        commands::device_browser::open_device_browser,
        commands::device_browser::browse_device,
        commands::device_browser::device_browser_ids,
        commands::device_browser::device_browser_selection,
        commands::device_browser::get_device_browser,
        commands::device_browser::close_device_browser,
        commands::device_browser::plan_device_browser_import,
        commands::device_browser::start_device_browser_import,
        commands::map::map_clusters,
        commands::map::map_bounds,
        commands::map::assets_in_bounds,
        commands::map::search_places,
        commands::map::place_for,
        commands::media::get_ffmpeg_status,
        commands::media::get_exiftool_status,
        commands::raw_formats::get_raw_formats_status,
        commands::raw_formats::clear_dng_cache,
        commands::media::get_playback,
        commands::media::prepare_proxy,
        commands::media::set_video_support,
        commands::media::prefetch_playback,
        commands::media::get_proxy_cache_size,
        commands::media::clear_proxy_cache,
        commands::media::get_video_prep_status,
        // Round 21 lane B
        commands::repair::list_repair_sources,
        commands::repair::start_repair_check,
        commands::repair::get_repair_status,
        commands::repair::get_repair_items,
        commands::repair::cancel_repair,
        commands::repair::start_repair_import,
        // Round 21 lane C
        commands::app::log_boot_gate,
        commands::catalog::get_develop_source,
        commands::catalog::set_develop_source,
        commands::catalog::list_draft_ids,
        // Round 22 lane B
        commands::repair::get_repair_outcomes,
        commands::repair::get_library_health,
        commands::repair::start_recover,
        commands::repair::get_recovered,
        commands::repair::discard_recovered,
        commands::repair::keep_recovered,
        commands::repair::undo_keep_recovered,
        // Round 22 lane C
        commands::external_apps::list_external_apps,
        commands::external_apps::add_custom_app,
        commands::external_apps::remove_custom_app,
        commands::external_apps::get_default_apps,
        commands::external_apps::set_default_app,
        commands::external_apps::set_app_send_mode,
        commands::external_apps::open_in_app,
        commands::plugin_themes::plugin_theme_css,
        commands::plugin_themes::themes_list, // Round 26 B1
        commands::plugin_themes::theme_tried,
        commands::plugin_themes::theme_inspect,
        commands::plugin_themes::theme_install,
        commands::plugin_themes::theme_remove,
        commands::plugin_themes::theme_try,
        commands::plugin_themes::theme_try_end,
        commands::plugin_themes::theme_keep,
        commands::plugin_themes::theme_apply,
        // Round 24 lane C
        commands::store::store_index,
        commands::store::store_install,
        commands::store::store_update,
        commands::store::store_readme,
        commands::store::store_status,
        commands::store::store_set_auto_update,
        commands::store::store_local_folder,
        commands::store::store_set_local_folder,
        commands::plugin_import::list_plugin_importers, // Round 23 lane A3
        commands::plugin_import::start_plugin_import,
        // Round 23 C1: plugins
        commands::plugins::list_plugins,
        commands::plugin_tools::plugin_tool_count,
        commands::plugins::inspect_plugin_package,
        commands::plugins::install_plugin,
        commands::plugins::set_plugin_enabled,
        commands::plugins::remove_plugin,
        commands::plugins::undo_remove_plugin,
        commands::plugins::set_plugin_setting,
        commands::plugins::set_plugin_developer_mode,
        commands::plugins::get_plugin_developer_mode,
        commands::plugins::plugin_readme,
        commands::plugins::load_dev_plugin,
        commands::plugins::reload_dev_plugin,
        commands::plugins::plugin_log,
        commands::plugins::plugin_ui_action,
        commands::plugins::plugin_ui_value,
        commands::plugins::take_pending_plugin_package,
        // Round 24 lane D
        commands::dialogs::dialog_pick,
        commands::dialogs::reveal_path,
        commands::dialogs::plugin_provider_choices,
        commands::dialogs::set_provider_choice,
        commands::dialogs::plugin_detect,
        commands::dialogs::plugin_locate,
        // Round 24 lane W
        commands::places::places_review,
        commands::places::places_review_event,
        commands::places::places_write,
        commands::places::places_write_cancel,
        commands::proposals::proposals_list,
        commands::proposals::proposals_counts,
        commands::proposals::proposals_reject,
        commands::proposals::proposals_restore,
        commands::proposals::proposals_accept,
        commands::proposals::proposals_undo_accept,
        commands::places::places_undo,
        commands::places::plugin_models_status,
        commands::places::plugin_models_download,
        commands::places::plugin_tool_run,
        #[cfg(debug_assertions)]
        commands::dev::register_probe,
    ]);

    builder
        .build(context)
        .expect("error while building tauri application")
        // Round 17: once the window has gone, a staged update installs (`updater::install_on_exit`).
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                crash_marker::clean_exit(); // Round 24 lane B
                updater::install_on_exit(app);
                window::on_exit();
            }
            // Round 23 C1: macOS opens a double-clicked .lumenplugin through an event.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                plugins::state::on_opened(app, &tauri::Manager::state::<std::sync::Arc<plugins::Plugins>>(app), urls);
            }
        });
}
