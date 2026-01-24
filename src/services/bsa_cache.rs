use anyhow::{Result, Context};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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
    /// Create a new SQLite cache in a temp file
    pub fn new() -> Result<Self> {
        // Create temp file for database
        let db_path = std::env::temp_dir().join(format!("ttw_bsa_cache_{}.db", std::process::id()));

        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to create SQLite cache at {}", db_path.display()))?;

        // Configure for performance
        conn.execute_batch(
            "PRAGMA journal_mode = OFF;
             PRAGMA synchronous = OFF;
             PRAGMA cache_size = 100000;
             PRAGMA temp_store = MEMORY;
             PRAGMA locking_mode = EXCLUSIVE;"
        ).context("Failed to configure SQLite pragmas")?;

        // Create cache table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS bsa_cache (
                bsa_path TEXT NOT NULL,
                file_path TEXT NOT NULL,
                data BLOB NOT NULL,
                PRIMARY KEY (bsa_path, file_path)
            )",
            [],
        ).context("Failed to create bsa_cache table")?;

        info!("Created SQLite BSA cache at {}", db_path.display());

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
            total_bytes: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Insert a file into the cache
    /// Returns the size of the data stored
    pub fn insert(&self, bsa_path: &Path, file_path: &str, data: &[u8]) -> Result<usize> {
        let bsa_str = bsa_path.to_string_lossy();
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO bsa_cache (bsa_path, file_path, data) VALUES (?1, ?2, ?3)",
            params![bsa_str.as_ref(), file_path, data],
        ).with_context(|| format!("Failed to cache {}:{}", bsa_str, file_path))?;

        self.total_bytes.fetch_add(data.len(), std::sync::atomic::Ordering::Relaxed);
        Ok(data.len())
    }

    /// Insert multiple files in a single transaction (much faster for bulk inserts)
    pub fn insert_batch(&self, bsa_path: &Path, files: Vec<(String, Vec<u8>)>) -> Result<(usize, usize)> {
        let bsa_str = bsa_path.to_string_lossy().to_string();
        let mut conn = self.conn.lock().unwrap();

        let tx = conn.transaction()
            .context("Failed to start transaction")?;

        let mut count = 0;
        let mut bytes = 0;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO bsa_cache (bsa_path, file_path, data) VALUES (?1, ?2, ?3)"
            ).context("Failed to prepare insert statement")?;

            for (file_path, data) in files {
                bytes += data.len();
                stmt.execute(params![&bsa_str, &file_path, &data])
                    .with_context(|| format!("Failed to cache {}:{}", bsa_str, file_path))?;
                count += 1;
            }
        }

        tx.commit().context("Failed to commit transaction")?;

        self.total_bytes.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
        Ok((count, bytes))
    }

    /// Get a file from the cache
    pub fn get(&self, bsa_path: &Path, file_path: &str) -> Result<Option<Vec<u8>>> {
        let bsa_str = bsa_path.to_string_lossy();
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare_cached(
            "SELECT data FROM bsa_cache WHERE bsa_path = ?1 AND file_path = ?2"
        ).context("Failed to prepare select statement")?;

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
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM bsa_cache", [])
            .context("Failed to clear cache")?;
        self.total_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Get total bytes stored in cache
    pub fn total_bytes(&self) -> usize {
        self.total_bytes.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get number of files in cache
    pub fn file_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM bsa_cache",
            [],
            |row| row.get(0),
        ).context("Failed to count cached files")?;
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
            warn!("Failed to remove temp cache file {}: {}", self.db_path.display(), e);
        } else {
            info!("Cleaned up SQLite cache: {}", self.db_path.display());
        }
    }
}
