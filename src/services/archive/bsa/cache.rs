use anyhow::{Context, Result};
use ba2::tes4::{Archive, ArchiveKey, DirectoryKey, File as BsaFile};
use ba2::{ByteSlice, Reader};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tracing::{info, warn};

/// SQLite-based cache for BSA file extraction
/// Stores extracted files on disk instead of RAM, dramatically reducing memory usage
pub struct BsaCache {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    /// Track total bytes stored for stats
    total_bytes: std::sync::atomic::AtomicUsize,
}

impl BsaCache {
    /// Create a new SQLite cache in the specified directory
    pub fn new_at(base_dir: PathBuf) -> Result<Self> {
        let db_path = base_dir.join(format!(".ttw_bsa_cache_{}.db", std::process::id()));

        // If a stale file exists from a crashed run, try to remove it
        if db_path.exists() {
            if let Err(e) = std::fs::remove_file(&db_path) {
                warn!(
                    "Failed to remove stale cache file {}: {}",
                    db_path.display(),
                    e
                );
            }
        }

        let conn = Connection::open(&db_path).with_context(|| {
            format!(
                "Failed to create SQLite cache at {}\n\
                Possible causes:\n\
                - Disk is full (check with 'df -h {}')\n\
                - Permission denied (check directory permissions)\n\
                - Filesystem is read-only",
                db_path.display(),
                base_dir.display()
            )
        })?;

        // Configure for performance with minimal memory footprint
        conn.execute_batch(
            "PRAGMA journal_mode = OFF;
             PRAGMA synchronous = OFF;
             PRAGMA cache_size = 1000;
             PRAGMA temp_store = FILE;
             PRAGMA locking_mode = EXCLUSIVE;
             PRAGMA mmap_size = 0;",
        )
        .context("Failed to configure SQLite pragmas")?;

        // Create cache table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS bsa_cache (
                bsa_path TEXT NOT NULL,
                file_path TEXT NOT NULL,
                data BLOB NOT NULL,
                PRIMARY KEY (bsa_path, file_path)
            )",
            [],
        )
        .context("Failed to create bsa_cache table")?;

        info!("Created SQLite BSA cache at {}", db_path.display());

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
            total_bytes: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("BSA cache connection lock poisoned"))
    }

    /// Insert a file into the cache
    /// Returns the size of the data stored
    pub fn insert(&self, bsa_path: &Path, file_path: &str, data: &[u8]) -> Result<usize> {
        let bsa_str = bsa_path.to_string_lossy();
        let conn = self.conn()?;

        conn.execute(
            "INSERT OR REPLACE INTO bsa_cache (bsa_path, file_path, data) VALUES (?1, ?2, ?3)",
            params![bsa_str.as_ref(), file_path, data],
        )
        .with_context(|| format!("Failed to cache {}:{}", bsa_str, file_path))?;

        self.total_bytes
            .fetch_add(data.len(), std::sync::atomic::Ordering::Relaxed);
        Ok(data.len())
    }

    /// Insert multiple files using a callback that yields files one at a time
    /// This avoids loading all files into RAM - each file is inserted and then dropped
    pub fn insert_streaming<F>(&self, bsa_path: &Path, mut producer: F) -> Result<(usize, usize)>
    where
        F: FnMut(&mut dyn FnMut(String, Vec<u8>) -> Result<()>) -> Result<()>,
    {
        let bsa_str = bsa_path.to_string_lossy().to_string();
        let mut conn = self.conn()?;

        let tx = conn.transaction().context("Failed to start transaction")?;

        let mut count = 0;
        let mut bytes = 0;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO bsa_cache (bsa_path, file_path, data) VALUES (?1, ?2, ?3)"
            ).context("Failed to prepare insert statement")?;

            let mut inserter = |file_path: String, data: Vec<u8>| -> Result<()> {
                bytes += data.len();
                stmt.execute(params![&bsa_str, &file_path, &data])
                    .with_context(|| format!("Failed to cache {}:{}", bsa_str, file_path))?;
                count += 1;
                Ok(())
            };

            producer(&mut inserter)?;
        }

        tx.commit().context("Failed to commit transaction")?;

        self.total_bytes
            .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
        Ok((count, bytes))
    }

    /// Get a file from the cache
    pub fn get(&self, bsa_path: &Path, file_path: &str) -> Result<Option<Vec<u8>>> {
        let bsa_str = bsa_path.to_string_lossy();
        let conn = self.conn()?;

        let mut stmt = conn
            .prepare_cached("SELECT data FROM bsa_cache WHERE bsa_path = ?1 AND file_path = ?2")
            .context("Failed to prepare select statement")?;

        let result = stmt.query_row(params![bsa_str.as_ref(), file_path], |row| {
            row.get::<_, Vec<u8>>(0)
        });

        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e).context("Failed to query cache"),
        }
    }

    /// Clear all cached data
    pub fn clear(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM bsa_cache", [])
            .context("Failed to clear cache")?;
        self.total_bytes
            .store(0, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Get total bytes stored in cache
    pub fn total_bytes(&self) -> usize {
        self.total_bytes.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get number of files in cache
    pub fn file_count(&self) -> Result<usize> {
        let conn = self.conn()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM bsa_cache", [], |row| row.get(0))
            .context("Failed to count cached files")?;
        Ok(count as usize)
    }

    /// Get database file size on disk
    pub fn db_size_bytes(&self) -> u64 {
        std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0)
    }
}

impl Drop for BsaCache {
    fn drop(&mut self) {
        // Clean up the temp database file
        if let Err(e) = std::fs::remove_file(&self.db_path) {
            warn!(
                "Failed to remove temp cache file {}: {}",
                self.db_path.display(),
                e
            );
        } else {
            info!("Cleaned up SQLite cache: {}", self.db_path.display());
        }
    }
}

/// Get file sizes from a BSA without extracting the data
/// Returns a map of file_path -> uncompressed size in bytes
pub fn get_bsa_file_sizes(bsa_path: &Path, file_paths: &[&str]) -> Result<HashMap<String, usize>> {
    let (archive, _): (Archive, _) = Archive::read(bsa_path)
        .with_context(|| format!("Failed to open BSA for size query: {}", bsa_path.display()))?;

    // Build set of needed paths (normalized to lowercase with backslashes)
    let needed: std::collections::HashSet<String> = file_paths
        .iter()
        .map(|p| p.replace('/', "\\").to_lowercase())
        .collect();

    // Build lookup for original casing
    let path_lookup: HashMap<String, &str> = file_paths
        .iter()
        .map(|p| (p.replace('/', "\\").to_lowercase(), *p))
        .collect();

    let mut sizes = HashMap::new();

    for (dir_key, folder) in archive.iter() {
        let dir_key: &ArchiveKey = dir_key;
        let dir_name = String::from_utf8_lossy(dir_key.name().as_bytes()).to_lowercase();

        for (file_key, file) in folder.iter() {
            let file_key: &DirectoryKey = file_key;
            let file: &BsaFile = file;
            let file_name = String::from_utf8_lossy(file_key.name().as_bytes()).to_lowercase();
            let full_path = if dir_name.is_empty() || dir_name == "." {
                file_name.clone()
            } else {
                format!("{}\\{}", dir_name, file_name)
            };

            if needed.contains(&full_path) {
                // Get file size - for compressed files, estimate ~3x compression ratio
                // This is conservative (real ratio is often 2-4x for textures)
                let size = if file.is_compressed() {
                    file.as_bytes().len() * 3 // Estimate decompressed size
                } else {
                    file.as_bytes().len()
                };

                let original_path = path_lookup
                    .get(&full_path)
                    .map(|s| s.to_string())
                    .unwrap_or(full_path);

                sizes.insert(original_path, size);
            }
        }
    }

    Ok(sizes)
}
