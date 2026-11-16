//! The remembered locale (DESIGN 8b.3): `{"locale": "de"}`, written atomically.

use std::path::Path;

/// The saved locale, if the file is there and well-formed.
pub(crate) fn read(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("locale")?.as_str().map(str::to_owned)
}

/// Writes the file through a temporary one, synced, and a rename (the directory synced
/// too on unix): a crash or power loss never leaves half a file. The temporary name is
/// unique per process and call, so two writers never share one.
pub(crate) fn write(path: &Path, locale: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent();
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir)?;
    }
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}.{n}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(
            serde_json::json!({ "locale": locale })
                .to_string()
                .as_bytes(),
        )?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    #[cfg(unix)]
    if let Some(dir) = dir.filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Given two threads saving at once into one directory, when both are done, then the
    /// file holds one of the two whole, and no temporary file is left behind (each write
    /// has its own temporary name, so neither renames the other's half-written one).
    #[test]
    fn concurrent_writes_leave_one_whole_file() {
        let dir = std::env::temp_dir().join(format!("persist-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("i18n-next.json");
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));
        let writers: Vec<_> = ["de", "fr"]
            .into_iter()
            .map(|l| {
                let (path, start) = (path.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    for _ in 0..50 {
                        super::write(&path, l).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        let saved = super::read(&path).unwrap();
        assert!(saved == "de" || saved == "fr");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from("i18n-next.json")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
