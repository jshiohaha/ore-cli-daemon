use chrono::Local;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::DAEMON_FILE_PATH;

pub fn log(message: &str) {
    let now = Local::now();
    let timestamp = now.format("%Y-%m-%d %H:%M:%S");

    // create a directory for the logs if it doesn't exist
    let log_dir = PathBuf::from(format!("{}/logs", DAEMON_FILE_PATH));
    create_dir_all(&log_dir).unwrap();

    // generate the filename based on the current hour
    let filename = format!("daemon_{}.log", now.format("%Y-%m-%d_%H"));
    let filepath = log_dir.join(filename);

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(filepath)
        .unwrap();

    writeln!(file, "{} - {}", timestamp, message).unwrap();
}
