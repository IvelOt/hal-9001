
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::events::{AppEvent, EventTx};

const SKIPPED_DIRS: &[&str] = &["/proc", "/sys", "/dev", "/run", "/tmp", "/sys/kernel/debug"];

fn is_skipped_dir(path: &Path) -> bool {
    SKIPPED_DIRS.iter().any(|s| path == Path::new(s))
}

const PROGRESS_ITEMS_INTERVAL: usize = 500;
const PROGRESS_TIME_INTERVAL: Duration = Duration::from_millis(100);

struct ScanProgress<'a> {
    tx: &'a EventTx,
    items_scanned: usize,
    total_bytes: u64,
    last_emit: Instant,
}

impl<'a> ScanProgress<'a> {
    fn new(tx: &'a EventTx) -> Self {
        Self {
            tx,
            items_scanned: 0,
            total_bytes: 0,
            last_emit: Instant::now(),
        }
    }

    fn record(&mut self, path: &Path, bytes: u64) {
        self.items_scanned += 1;
        self.total_bytes += bytes;
        if self.items_scanned % PROGRESS_ITEMS_INTERVAL == 0
            || self.last_emit.elapsed() >= PROGRESS_TIME_INTERVAL
        {
            self.last_emit = Instant::now();
            let _ = self.tx.send(AppEvent::StorageAnalyzerProgress {
                current_item: path.display().to_string(),
                items_scanned: self.items_scanned,
                total_bytes: self.total_bytes,
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiskUsageItem {
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,

    pub percentage: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiskUsageSnapshot {
    pub current_path: PathBuf,

    pub total_bytes: u64,
    pub items: Vec<DiskUsageItem>,
    pub is_scanning: bool,
}

fn dir_size_recursive(path: &Path, progress: &mut ScanProgress) -> u64 {
    if is_skipped_dir(path) {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let entry_path = entry.path();
        let size = if meta.is_dir() {
            dir_size_recursive(&entry_path, progress)
        } else {
            meta.len()
        };
        total += size;
        progress.record(&entry_path, if meta.is_dir() { 0 } else { size });
    }
    total
}

fn friendly_scan_error(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => {
            "Permissão negada para acessar este diretório".to_string()
        }
        std::io::ErrorKind::NotFound => "Diretório não encontrado".to_string(),
        _ => format!("Falha ao escanear: {err}"),
    }
}

pub fn scan_dir(path: &Path, tx: &EventTx) -> std::io::Result<DiskUsageSnapshot> {
    let entries = std::fs::read_dir(path)?;
    let mut progress = ScanProgress::new(tx);

    let mut items: Vec<DiskUsageItem> = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let (is_dir, size_bytes) = if meta.is_dir() {
            if is_skipped_dir(&entry_path) {
                (true, 0)
            } else {
                (true, dir_size_recursive(&entry_path, &mut progress))
            }
        } else {
            (false, meta.len())
        };
        progress.record(&entry_path, if is_dir { 0 } else { size_bytes });
        items.push(DiskUsageItem {
            name,
            is_dir,
            size_bytes,
            percentage: 0.0,
        });
    }

    items.sort_by_key(|i| std::cmp::Reverse(i.size_bytes));

    let max_size = items.iter().map(|i| i.size_bytes).max().unwrap_or(0);
    if max_size > 0 {
        for item in &mut items {
            item.percentage = (item.size_bytes as f64 / max_size as f64 * 100.0) as f32;
        }
    }

    let total_bytes = items.iter().map(|i| i.size_bytes).sum();

    Ok(DiskUsageSnapshot {
        current_path: path.to_path_buf(),
        total_bytes,
        items,
        is_scanning: false,
    })
}

pub async fn scan(path: PathBuf, tx: EventTx) {
    let result = tokio::task::spawn_blocking({
        let tx = tx.clone();
        move || scan_dir(&path, &tx)
    })
    .await;
    match result {
        Ok(Ok(snapshot)) => {
            let _ = tx.send(AppEvent::StorageAnalyzerSnapshot(Box::new(snapshot)));
        }
        Ok(Err(e)) => {
            let _ = tx.send(AppEvent::StorageAnalyzerError {
                path: PathBuf::new(),
                message: friendly_scan_error(&e),
            });
        }
        Err(e) => {
            let _ = tx.send(AppEvent::StorageAnalyzerError {
                path: PathBuf::new(),
                message: e.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tx() -> EventTx {
        tokio::sync::mpsc::unbounded_channel().0
    }

    #[test]
    fn scans_files_and_dirs_sorted_by_size_desc() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("small.txt"), vec![0u8; 10]).unwrap();
        std::fs::write(tmp.path().join("big.txt"), vec![0u8; 1000]).unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        std::fs::write(tmp.path().join("subdir/nested.txt"), vec![0u8; 500]).unwrap();

        let snap = scan_dir(tmp.path(), &test_tx()).unwrap();
        assert_eq!(snap.items.len(), 3);
        assert_eq!(snap.items[0].name, "big.txt");
        assert_eq!(snap.items[0].size_bytes, 1000);
        assert_eq!(snap.items[0].percentage, 100.0);

        let subdir = snap.items.iter().find(|i| i.name == "subdir").unwrap();
        assert!(subdir.is_dir);
        assert_eq!(subdir.size_bytes, 500);

        assert_eq!(snap.total_bytes, 1510);
        assert!(!snap.is_scanning);
    }

    #[test]
    fn empty_dir_yields_no_items() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = scan_dir(tmp.path(), &test_tx()).unwrap();
        assert!(snap.items.is_empty());
        assert_eq!(snap.total_bytes, 0);
    }

    #[test]
    fn missing_dir_errors() {
        assert!(scan_dir(Path::new("/nonexistent/path/for/hal9001/tests"), &test_tx()).is_err());
    }

    #[test]
    fn skips_virtual_filesystem_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("proc")).unwrap();
        std::fs::write(tmp.path().join("real.txt"), vec![0u8; 100]).unwrap();

        assert!(is_skipped_dir(Path::new("/proc")));
        assert!(is_skipped_dir(Path::new("/sys")));
        assert!(is_skipped_dir(Path::new("/dev")));
        assert!(is_skipped_dir(Path::new("/run")));
        assert!(is_skipped_dir(Path::new("/tmp")));
        assert!(is_skipped_dir(Path::new("/sys/kernel/debug")));
        assert!(!is_skipped_dir(&tmp.path().join("proc")));

        let snap = scan_dir(tmp.path(), &test_tx()).unwrap();
        assert_eq!(snap.total_bytes, 100);
    }

    #[test]
    fn emits_progress_events_during_scan() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(tmp.path().join(format!("f{i}.txt")), vec![0u8; 10]).unwrap();
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let snap = scan_dir(tmp.path(), &tx).unwrap();
        assert_eq!(snap.items.len(), 5);
        while let Ok(AppEvent::StorageAnalyzerProgress { items_scanned, .. }) = rx.try_recv() {
            assert!(items_scanned > 0);
        }
    }
}
