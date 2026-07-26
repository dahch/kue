use tauri::Manager;

mod audio;
mod db;
mod rag;
mod stt;
mod types;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    db::register_vec_extension();

    // Clean up any WAV temp dirs left from a previous crashed session
    audio::capture::AudioCapture::cleanup_orphaned_temp_dirs();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let database = db::init_db(app)?;
            app.manage(database);

            let recordings_dir = app.path().app_data_dir()?.join("recordings");
            app.manage(audio::capture::AudioCapture::new(recordings_dir));

            let model = rag::embeddings::load_embedding_model()?;
            app.manage(std::sync::Mutex::new(model));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::get_db_status,
            audio::capture::toggle_audio_capture,
            rag::indexer::index_folder_cmd,
            rag::indexer::search_context,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
