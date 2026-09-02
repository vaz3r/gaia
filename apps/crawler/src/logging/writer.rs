use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

pub struct AsyncWriter {
    rx: tokio::sync::mpsc::Receiver<String>,
    dir: PathBuf,
    file_max_bytes: u64,
    total_max_bytes: u64,
    flush_interval_ms: u64,
    batch_size: usize,
    max_file_age_secs: u64,
    shutdown_rx: oneshot::Receiver<()>,
}

impl AsyncWriter {
    pub fn new(
        rx: tokio::sync::mpsc::Receiver<String>,
        dir: PathBuf,
        file_max_bytes: u64,
        total_max_bytes: u64,
        flush_interval_ms: u64,
        batch_size: usize,
        max_file_age_secs: u64,
        shutdown_rx: oneshot::Receiver<()>,
    ) -> Self {
        Self {
            rx,
            dir,
            file_max_bytes,
            total_max_bytes,
            flush_interval_ms,
            batch_size,
            max_file_age_secs,
            shutdown_rx,
        }
    }

    pub async fn run(mut self) {
        let _ = fs::create_dir_all(&self.dir);

        let mut buffer: Vec<String> = Vec::with_capacity(self.batch_size.min(8192));
        let mut bytes_written: u64 = 0;
        let mut file_start = Instant::now();
        let (path, file) = match create_new_file(&self.dir) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[writer] FATAL: failed to create initial log file: {e}");
                return;
            }
        };
        let mut current_path = path;
        let mut writer = BufWriter::new(file);
        let mut flush_interval =
            tokio::time::interval(Duration::from_millis(self.flush_interval_ms));
        flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                biased;

                _ = &mut self.shutdown_rx => {
                    drain_and_flush(&mut self.rx, &mut writer, &mut buffer);
                    let _ = writer.flush();
                    break;
                }
                msg = self.rx.recv() => {
                    match msg {
                        Some(line) => {
                            buffer.push(line);
                            if buffer.len() >= self.batch_size {
                                flush_and_maybe_rotate(&mut writer, &mut buffer, &mut bytes_written, &mut file_start, &mut current_path, &self);
                            }
                        }
                        None => {
                            drain_and_flush(&mut self.rx, &mut writer, &mut buffer);
                            let _ = writer.flush();
                            break;
                        }
                    }
                    if !std::path::Path::new(&current_path).exists() {
                        if !buffer.is_empty() {
                            let _ = writer.flush();
                        }
                        match create_new_file(&self.dir) {
                            Ok((new_path, new_file)) => {
                                let new_writer = BufWriter::new(new_file);
                                let old = std::mem::replace(&mut writer, new_writer);
                                drop(old);
                                current_path = new_path;
                                bytes_written = 0;
                                file_start = Instant::now();
                            }
                            Err(e) => {
                                eprintln!("[writer] failed to recreate log file after deletion: {e}");
                            }
                        }
                    }
                }
                _ = flush_interval.tick() => {
                    if !buffer.is_empty() {
                        flush_and_maybe_rotate(&mut writer, &mut buffer, &mut bytes_written, &mut file_start, &mut current_path, &self);
                    }
                    if !std::path::Path::new(&current_path).exists() {
                        match create_new_file(&self.dir) {
                            Ok((new_path, new_file)) => {
                                let new_writer = BufWriter::new(new_file);
                                let old = std::mem::replace(&mut writer, new_writer);
                                drop(old);
                                current_path = new_path;
                                bytes_written = 0;
                                file_start = Instant::now();
                            }
                            Err(e) => {
                                eprintln!("[writer] failed to recreate log file after deletion: {e}");
                            }
                        }
                    }
                }
            }
        }
    }
}

fn drain_and_flush(
    rx: &mut tokio::sync::mpsc::Receiver<String>,
    writer: &mut BufWriter<File>,
    buffer: &mut Vec<String>,
) {
    while let Ok(line) = rx.try_recv() {
        buffer.push(line);
    }
    for line in buffer.drain(..) {
        let _ = writeln!(writer, "{}", line);
    }
    let _ = writer.flush();
}

fn flush_and_maybe_rotate(
    writer: &mut BufWriter<File>,
    buffer: &mut Vec<String>,
    bytes_written: &mut u64,
    file_start: &mut Instant,
    current_path: &mut String,
    ctx: &AsyncWriter,
) {
    for line in buffer.drain(..) {
        let _ = writeln!(writer, "{}", line);
        *bytes_written += line.len() as u64 + 1;
    }
    let _ = writer.flush();

    let should_rotate = *bytes_written >= ctx.file_max_bytes
        || file_start.elapsed() >= Duration::from_secs(ctx.max_file_age_secs);

    if should_rotate {
        rotate(writer, bytes_written, file_start, current_path, ctx);
    }
}

fn rotate(
    writer: &mut BufWriter<File>,
    bytes_written: &mut u64,
    file_start: &mut Instant,
    current_path: &mut String,
    ctx: &AsyncWriter,
) {
    let old_path = current_path.clone();

    let (new_path, new_file) = match create_new_file(&ctx.dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[writer] failed to create rotated log file: {e}");
            return;
        }
    };
    let new_writer = BufWriter::new(new_file);

    let old = std::mem::replace(writer, new_writer);
    drop(old);

    if let Ok(f) = File::open(&old_path) {
        let _ = f.sync_all();
        drop(f);
    }

    *current_path = new_path;
    *bytes_written = 0;
    *file_start = Instant::now();

    cleanup_old_files(&ctx.dir, ctx.total_max_bytes);
}

fn cleanup_old_files(dir: &PathBuf, total_max_bytes: u64) {
    let entries: Vec<_> = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((e.path(), meta.len()))
        })
        .collect();

    let total: u64 = entries.iter().map(|(_, len)| len).sum();

    if total <= total_max_bytes {
        return;
    }

    let mut sorted = entries;
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut remaining = total;
    for (path, len) in &sorted {
        if remaining <= total_max_bytes {
            break;
        }
        let _ = fs::remove_file(path);
        remaining -= len;
    }
}

fn create_new_file(dir: &PathBuf) -> Result<(String, File), std::io::Error> {
    let now = chrono::Utc::now();
    let filename = format!("crawler-{}.jsonl", now.format("%Y-%m-%dT%H-%M-%SZ"));
    let path = dir.join(&filename);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    Ok((path.to_string_lossy().to_string(), file))
}
