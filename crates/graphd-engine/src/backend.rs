use std::path::Path;
use std::sync::Arc;

use crate::error::GraphdError;

/// LadybugDB graph database backend.
///
/// The database is stored as `Arc<lbug::Database>` so it can be shared with
/// HA coordinators (hakuzu) that need direct database access for replication.
pub struct Backend {
    db: Arc<lbug::Database>,
}

impl Backend {
    /// Open or create a database at the given directory path with default config.
    pub fn open(path: &Path) -> Result<Self, GraphdError> {
        Self::open_with_config(path, lbug::SystemConfig::default())
    }

    /// Open or create a database with memory and thread limits.
    ///
    /// This is the preferred constructor for multi-tenant environments.
    pub fn open_tenant(path: &Path, memory_mb: u64, max_threads: u64) -> Result<Self, GraphdError> {
        let config = lbug::SystemConfig::default()
            .buffer_pool_size(memory_mb * 1024 * 1024)
            .max_num_threads(max_threads);
        Self::open_with_config(path, config)
    }

    /// Open or create a database at the given directory path with custom config.
    pub fn open_with_config(path: &Path, config: lbug::SystemConfig) -> Result<Self, GraphdError> {
        let db = lbug::Database::new(path, config)
            .map_err(|e| GraphdError::DatabaseError(format!("Failed to open database: {e}")))?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Wrap a pre-existing shared database handle.
    ///
    /// Use this when the database is created externally and shared with an HA
    /// coordinator (e.g. hakuzu) that also needs `Arc<lbug::Database>`.
    pub fn from_database(db: Arc<lbug::Database>) -> Self {
        Self { db }
    }

    /// Get a shared reference to the underlying database.
    ///
    /// Use this to pass the same database to hakuzu's builder via `.database(db)`.
    pub fn database(&self) -> &Arc<lbug::Database> {
        &self.db
    }

    /// Create a new connection to the database.
    pub fn connection(&self) -> Result<lbug::Connection<'_>, GraphdError> {
        lbug::Connection::new(&self.db)
            .map_err(|e| GraphdError::DatabaseError(format!("Connection failed: {e}")))
    }
}
