use tauri::Manager;

mod audio;
mod db;
mod rag;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    db::register_vec_extension();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let database = db::init_db(app)?;
            app.manage(database);

            let recordings_dir = app.path().app_data_dir()?.join("recordings");
            std::fs::create_dir_all(&recordings_dir).ok();
            app.manage(audio::capture::AudioCapture::new(recordings_dir));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::get_db_status,
            audio::capture::toggle_audio_capture
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
