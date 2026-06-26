use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use super::cold_types::{ColdStartMode, EvictionResult};

/// Compute the default filler size: max(512 MB, dataset_size * 2).
pub fn default_filler_size(dataset_size_bytes: u64) -> u64 {
    const MIN_FILLER: u64 = 512 * 1024 * 1024; // 512 MB
    MIN_FILLER.max(dataset_size_bytes * 2)
}

/// Calculate the total size of all files in a directory (non-recursive).
pub fn compute_dataset_size(dir: &Path) -> u64 {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    let mut total = 0_u64;
    for entry in entries.flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

/// Perform cache eviction based on the selected mode.
pub fn evict_cache(
    mode: ColdStartMode,
    filler_size_bytes: u64,
    dataset_size_bytes: u64,
) -> EvictionResult {
    match mode {
        ColdStartMode::ProcessCold => EvictionResult {
            requested: false,
            method: mode,
            succeeded: true,
            duration_ms: 0.0,
            filler_size_bytes: 0,
            dataset_size_bytes,
            notes: vec!["OS page cache eviction was not requested.".to_owned()],
        },
        ColdStartMode::OsBestEffort => {
            evict_best_effort(mode, filler_size_bytes, dataset_size_bytes)
        }
        ColdStartMode::LinuxDropCache => evict_linux_drop_cache(mode, dataset_size_bytes),
    }
}

fn evict_best_effort(
    mode: ColdStartMode,
    filler_size_bytes: u64,
    dataset_size_bytes: u64,
) -> EvictionResult {
    let start = Instant::now();
    let temp_dir = std::env::temp_dir();
    let filler_path = temp_dir.join(format!("phs-cold-cache-{}.bin", std::process::id()));
    let chunk_size = 1024 * 1024_usize; // 1 MB
    let mut chunk = vec![0u8; chunk_size];
    for (i, byte) in chunk.iter_mut().enumerate() {
        *byte = (i & 0xFF) as u8 ^ 0xAA;
    }

    let result = (|| -> Result<(), String> {
        // Write filler
        let mut file = fs::File::create(&filler_path)
            .map_err(|e| format!("Failed to create filler file: {e}"))?;
        let mut written = 0_u64;
        while written < filler_size_bytes {
            let len = chunk_size.min((filler_size_bytes - written) as usize);
            file.write_all(&chunk[..len])
                .map_err(|e| format!("Failed to write filler: {e}"))?;
            written += len as u64;
        }
        file.sync_all()
            .map_err(|e| format!("Failed to sync filler: {e}"))?;
        drop(file);

        // Read filler back to force pages into cache
        let mut file = fs::File::open(&filler_path)
            .map_err(|e| format!("Failed to open filler for read: {e}"))?;
        let mut read_buf = vec![0u8; chunk_size];
        loop {
            let n = file
                .read(&mut read_buf)
                .map_err(|e| format!("Failed to read filler: {e}"))?;
            if n == 0 {
                break;
            }
        }
        drop(file);

        // Remove filler
        let _ = fs::remove_file(&filler_path);
        Ok(())
    })();

    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Ok(()) => {
            let ratio = if dataset_size_bytes > 0 {
                filler_size_bytes as f64 / dataset_size_bytes as f64
            } else {
                0.0
            };
            EvictionResult {
                requested: true,
                method: mode,
                succeeded: true,
                duration_ms,
                filler_size_bytes,
                dataset_size_bytes,
                notes: vec![format!(
                    "Filled OS file cache with {:.1} MB non-zero filler (filler/dataset = {ratio:.1}x). Best-effort perturbation.",
                    filler_size_bytes as f64 / (1024.0 * 1024.0)
                )],
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&filler_path);
            EvictionResult {
                requested: true,
                method: mode,
                succeeded: false,
                duration_ms,
                filler_size_bytes,
                dataset_size_bytes,
                notes: vec![format!("Best-effort cache perturbation failed: {error}")],
            }
        }
    }
}

fn evict_linux_drop_cache(mode: ColdStartMode, dataset_size_bytes: u64) -> EvictionResult {
    let start = Instant::now();

    #[cfg(target_os = "linux")]
    {
        let sync_result = std::process::Command::new("sync").status();
        if let Err(e) = sync_result {
            return EvictionResult {
                requested: true,
                method: mode,
                succeeded: false,
                duration_ms: start.elapsed().as_secs_f64() * 1000.0,
                filler_size_bytes: 0,
                dataset_size_bytes,
                notes: vec![format!("Could not run sync: {e}")],
            };
        }
        match fs::write("/proc/sys/vm/drop_caches", "3\n") {
            Ok(()) => EvictionResult {
                requested: true,
                method: mode,
                succeeded: true,
                duration_ms: start.elapsed().as_secs_f64() * 1000.0,
                filler_size_bytes: 0,
                dataset_size_bytes,
                notes: vec!["Wrote 3 to /proc/sys/vm/drop_caches after sync.".to_owned()],
            },
            Err(e) => EvictionResult {
                requested: true,
                method: mode,
                succeeded: false,
                duration_ms: start.elapsed().as_secs_f64() * 1000.0,
                filler_size_bytes: 0,
                dataset_size_bytes,
                notes: vec![format!("Could not drop Linux page cache: {e}")],
            },
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        EvictionResult {
            requested: true,
            method: mode,
            succeeded: false,
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
            filler_size_bytes: 0,
            dataset_size_bytes,
            notes: vec!["linux-drop-cache mode is only available on Linux.".to_owned()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_cold_no_eviction() {
        let result = evict_cache(ColdStartMode::ProcessCold, 0, 1000);
        assert!(!result.requested);
        assert!(result.succeeded);
        assert_eq!(result.filler_size_bytes, 0);
    }

    #[test]
    fn best_effort_creates_and_cleans_up() {
        // Use a small filler (2 MB) for test speed.
        let filler = 2 * 1024 * 1024;
        let result = evict_cache(ColdStartMode::OsBestEffort, filler, 1000);
        assert!(result.requested);
        assert!(result.succeeded);
        assert_eq!(result.filler_size_bytes, filler);
        assert!(result.duration_ms >= 0.0);
        // Filler file should be cleaned up.
        let filler_path =
            std::env::temp_dir().join(format!("phs-cold-cache-{}.bin", std::process::id()));
        assert!(!filler_path.exists());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn linux_drop_cache_on_non_linux() {
        let result = evict_cache(ColdStartMode::LinuxDropCache, 0, 1000);
        assert!(result.requested);
        assert!(!result.succeeded);
        assert!(result.notes[0].contains("only available on Linux"));
    }

    #[test]
    fn default_filler_size_minimum() {
        // Small dataset: should use 512 MB minimum.
        assert_eq!(default_filler_size(100), 512 * 1024 * 1024);
    }

    #[test]
    fn default_filler_size_scales() {
        // 400 MB dataset: 2x = 800 MB > 512 MB.
        let ds = 400 * 1024 * 1024;
        assert_eq!(default_filler_size(ds), ds * 2);
    }
}
