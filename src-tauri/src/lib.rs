use std::sync::Arc;

use tauri::Manager;

mod audio;
mod classifier;
mod db;
mod orchestrator;
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

            let model = Arc::new(std::sync::Mutex::new(
                rag::embeddings::load_embedding_model()?,
            ));
            app.manage(model.clone());

            let scheduler = Arc::new(orchestrator::HintScheduler::new());
            app.manage(scheduler.clone());

            let (hint_tx, hint_rx) = std::sync::mpsc::channel();
            let hint_job_tx: orchestrator::HintJobSender = Arc::new(hint_tx);
            app.manage(hint_job_tx);

            let db_for_worker = db::Database::clone(app.state::<db::Database>().inner());
            orchestrator::worker::start_hint_worker(
                hint_rx,
                app.handle().clone(),
                db_for_worker,
                model,
                scheduler,
            );

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::get_db_status,
            audio::capture::toggle_audio_capture,
            rag::indexer::index_folder_cmd,
            rag::indexer::search_context,
            classifier::classify_text,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
