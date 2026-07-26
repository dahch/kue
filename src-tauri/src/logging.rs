use std::fs;
use std::io::Write;
use std::sync::Mutex;

pub struct Logger {
    file: Mutex<fs::File>,
}

impl Logger {
    pub fn init(app_data_dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let logs_dir = app_data_dir.join("logs");
        fs::create_dir_all(&logs_dir)?;

        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let log_path = logs_dir.join(format!("kue_{}.log", timestamp));

        // Rotate: keep at most 5 log files, delete oldest
        let mut entries: Vec<_> = fs::read_dir(&logs_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "log"))
            .collect();
        entries.sort_by_key(|e| e.path());

        while entries.len() >= 5 {
            if let Some(oldest) = entries.first() {
                let _ = fs::remove_file(oldest.path());
                entries.remove(0);
            }
        }

        let file = fs::File::create(&log_path)?;
        let logger = Logger {
            file: Mutex::new(file),
        };

        let prefix = log_path.to_string_lossy().to_string();

        let result = log::set_boxed_logger(Box::new(logger));
        if result.is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
            log::info!("Log initialized at {}", prefix);
            log::info!("Kue v{}", env!("CARGO_PKG_VERSION"));
        }

        eprintln!("[kue] Logging to {}", prefix);

        Ok(())
    }

    fn write_log(&self, level: &str, target: &str, args: std::fmt::Arguments<'_>) {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "[{}] [{}] [{}] {}", now, level, target, args);
            let _ = file.flush();
        }
    }
}

impl log::Log for Logger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        self.write_log(&record.level().to_string(), record.target(), *record.args());
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}
