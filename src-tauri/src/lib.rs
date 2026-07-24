use tauri::Manager;

mod db;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    db::register_vec_extension();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let database = db::init_db(app)?;
            app.manage(database);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![db::get_db_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
