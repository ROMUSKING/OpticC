use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::Path;

const FILE_HASHES_TABLE: TableDefinition<&[u8; 32], &str> = TableDefinition::new("file_hashes");
const MACROS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("macros");

#[derive(Debug)]
pub enum DbError {
    Redb(redb::Error),
    Io(std::io::Error),
    Transaction(redb::TransactionError),
    Table(redb::TableError),
    Storage(redb::StorageError),
    Commit(redb::CommitError),
    Database(redb::DatabaseError),
}

impl From<redb::Error> for DbError {
    fn from(err: redb::Error) -> Self {
        DbError::Redb(err)
    }
}

impl From<std::io::Error> for DbError {
    fn from(err: std::io::Error) -> Self {
        DbError::Io(err)
    }
}

impl From<redb::TransactionError> for DbError {
    fn from(err: redb::TransactionError) -> Self {
        DbError::Transaction(err)
    }
}

impl From<redb::TableError> for DbError {
    fn from(err: redb::TableError) -> Self {
        DbError::Table(err)
    }
}

impl From<redb::StorageError> for DbError {
    fn from(err: redb::StorageError) -> Self {
        DbError::Storage(err)
    }
}

impl From<redb::CommitError> for DbError {
    fn from(err: redb::CommitError) -> Self {
        DbError::Commit(err)
    }
}

impl From<redb::DatabaseError> for DbError {
    fn from(err: redb::DatabaseError) -> Self {
        DbError::Database(err)
    }
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Redb(e) => write!(f, "redb error: {}", e),
            DbError::Io(e) => write!(f, "IO error: {}", e),
            DbError::Transaction(e) => write!(f, "transaction error: {}", e),
            DbError::Table(e) => write!(f, "table error: {}", e),
            DbError::Storage(e) => write!(f, "storage error: {}", e),
            DbError::Commit(e) => write!(f, "commit error: {}", e),
            DbError::Database(e) => write!(f, "database error: {}", e),
        }
    }
}

impl std::error::Error for DbError {}

pub type Result<T> = std::result::Result<T, DbError>;

pub struct OpticDb {
    db: Database,
}

impl OpticDb {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = Database::create(path).map_err(DbError::from)?;
        let write_txn = db.begin_write().map_err(DbError::from)?;
        {
            let _ = write_txn
                .open_table(FILE_HASHES_TABLE)
                .map_err(DbError::from)?;
            let _ = write_txn.open_table(MACROS_TABLE).map_err(DbError::from)?;
        }
        write_txn.commit().map_err(DbError::from)?;
        Ok(Self { db })
    }

    pub fn insert_file_hash(&self, hash: &[u8; 32], file_path: &str) -> Result<()> {
        let write_txn = self.db.begin_write().map_err(DbError::from)?;
        {
            let mut table = write_txn
                .open_table(FILE_HASHES_TABLE)
                .map_err(DbError::from)?;
            table.insert(hash, file_path).map_err(DbError::from)?;
        }
        write_txn.commit().map_err(DbError::from)?;
        Ok(())
    }

    pub fn get_file_path_by_hash(&self, hash: &[u8; 32]) -> Result<Option<String>> {
        let read_txn = self.db.begin_read().map_err(DbError::from)?;
        let table = read_txn
            .open_table(FILE_HASHES_TABLE)
            .map_err(DbError::from)?;
        match table.get(hash).map_err(DbError::from)? {
            Some(access_guard) => Ok(Some(access_guard.value().to_string())),
            None => Ok(None),
        }
    }

    pub fn contains_file_hash(&self, hash: &[u8; 32]) -> Result<bool> {
        let read_txn = self.db.begin_read().map_err(DbError::from)?;
        let table = read_txn
            .open_table(FILE_HASHES_TABLE)
            .map_err(DbError::from)?;
        Ok(table.get(hash).map_err(DbError::from)?.is_some())
    }

    pub fn insert_macro(&self, name: &str, definition: &str) -> Result<()> {
        let write_txn = self.db.begin_write().map_err(DbError::from)?;
        {
            let mut table = write_txn.open_table(MACROS_TABLE).map_err(DbError::from)?;
            table.insert(name, definition).map_err(DbError::from)?;
        }
        write_txn.commit().map_err(DbError::from)?;
        Ok(())
    }

    pub fn get_macro(&self, name: &str) -> Result<Option<String>> {
        let read_txn = self.db.begin_read().map_err(DbError::from)?;
        let table = read_txn.open_table(MACROS_TABLE).map_err(DbError::from)?;
        match table.get(name).map_err(DbError::from)? {
            Some(access_guard) => Ok(Some(access_guard.value().to_string())),
            None => Ok(None),
        }
    }

    pub fn remove_file_hash(&self, hash: &[u8; 32]) -> Result<Option<String>> {
        let write_txn = self.db.begin_write().map_err(DbError::from)?;
        let old_value = {
            let mut table = write_txn
                .open_table(FILE_HASHES_TABLE)
                .map_err(DbError::from)?;
            let x = if let Some(g) = table.remove(hash).map_err(DbError::from)? {
                Some(g.value().to_string())
            } else {
                None
            };
            x
        };
        write_txn.commit().map_err(DbError::from)?;
        Ok(old_value)
    }

    pub fn remove_macro(&self, name: &str) -> Result<Option<String>> {
        let write_txn = self.db.begin_write().map_err(DbError::from)?;
        let old_value = {
            let mut table = write_txn.open_table(MACROS_TABLE).map_err(DbError::from)?;
            let x = if let Some(g) = table.remove(name).map_err(DbError::from)? {
                Some(g.value().to_string())
            } else {
                None
            };
            x
        };
        write_txn.commit().map_err(DbError::from)?;
        Ok(old_value)
    }

    pub fn file_hash_count(&self) -> Result<u64> {
        let read_txn = self.db.begin_read().map_err(DbError::from)?;
        let table = read_txn
            .open_table(FILE_HASHES_TABLE)
            .map_err(DbError::from)?;
        Ok(table.len().map_err(DbError::from)?)
    }

    pub fn macro_count(&self) -> Result<u64> {
        let read_txn = self.db.begin_read().map_err(DbError::from)?;
        let table = read_txn.open_table(MACROS_TABLE).map_err(DbError::from)?;
        Ok(table.len().map_err(DbError::from)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db() -> (OpticDb, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.redb");
        let db = OpticDb::new(&db_path).unwrap();
        (db, temp_dir)
    }

    #[test]
    fn test_insert_and_get_file_hash() {
        let (db, _temp_dir) = create_test_db();
        let hash = [1u8; 32];
        let path = "/usr/include/stdio.h";

        db.insert_file_hash(&hash, path).unwrap();
        let result = db.get_file_path_by_hash(&hash).unwrap();
        assert_eq!(result, Some(path.to_string()));
    }

    #[test]
    fn test_contains_file_hash() {
        let (db, _temp_dir) = create_test_db();
        let hash = [2u8; 32];

        assert!(!db.contains_file_hash(&hash).unwrap());
        db.insert_file_hash(&hash, "/test.h").unwrap();
        assert!(db.contains_file_hash(&hash).unwrap());
    }

    #[test]
    fn test_get_nonexistent_file_hash() {
        let (db, _temp_dir) = create_test_db();
        let hash = [3u8; 32];
        let result = db.get_file_path_by_hash(&hash).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_insert_and_get_macro() {
        let (db, _temp_dir) = create_test_db();
        db.insert_macro("MAX", "(a > b ? a : b)").unwrap();
        let result = db.get_macro("MAX").unwrap();
        assert_eq!(result, Some("(a > b ? a : b)".to_string()));
    }

    #[test]
    fn test_get_nonexistent_macro() {
        let (db, _temp_dir) = create_test_db();
        let result = db.get_macro("UNDEFINED").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_overwrite_file_hash() {
        let (db, _temp_dir) = create_test_db();
        let hash = [4u8; 32];

        db.insert_file_hash(&hash, "/old/path.h").unwrap();
        db.insert_file_hash(&hash, "/new/path.h").unwrap();

        let result = db.get_file_path_by_hash(&hash).unwrap();
        assert_eq!(result, Some("/new/path.h".to_string()));
    }

    #[test]
    fn test_overwrite_macro() {
        let (db, _temp_dir) = create_test_db();

        db.insert_macro("DEBUG", "1").unwrap();
        db.insert_macro("DEBUG", "0").unwrap();

        let result = db.get_macro("DEBUG").unwrap();
        assert_eq!(result, Some("0".to_string()));
    }

    #[test]
    fn test_remove_file_hash() {
        let (db, _temp_dir) = create_test_db();
        let hash = [5u8; 32];

        db.insert_file_hash(&hash, "/test.h").unwrap();
        let removed = db.remove_file_hash(&hash).unwrap();
        assert_eq!(removed, Some("/test.h".to_string()));
        assert!(!db.contains_file_hash(&hash).unwrap());
    }

    #[test]
    fn test_remove_macro() {
        let (db, _temp_dir) = create_test_db();

        db.insert_macro("REMOVE_ME", "value").unwrap();
        let removed = db.remove_macro("REMOVE_ME").unwrap();
        assert_eq!(removed, Some("value".to_string()));
        assert_eq!(db.get_macro("REMOVE_ME").unwrap(), None);
    }

    #[test]
    fn test_counts() {
        let (db, _temp_dir) = create_test_db();

        assert_eq!(db.file_hash_count().unwrap(), 0);
        assert_eq!(db.macro_count().unwrap(), 0);

        db.insert_file_hash(&[1u8; 32], "/a.h").unwrap();
        db.insert_file_hash(&[2u8; 32], "/b.h").unwrap();
        db.insert_macro("A", "1").unwrap();

        assert_eq!(db.file_hash_count().unwrap(), 2);
        assert_eq!(db.macro_count().unwrap(), 1);
    }

    #[test]
    fn test_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("persist.redb");

        {
            let db = OpticDb::new(&db_path).unwrap();
            db.insert_file_hash(&[10u8; 32], "/persist.h").unwrap();
            db.insert_macro("PERSIST", "true").unwrap();
        }

        let db = OpticDb::new(&db_path).unwrap();
        assert_eq!(
            db.get_file_path_by_hash(&[10u8; 32]).unwrap(),
            Some("/persist.h".to_string())
        );
        assert_eq!(db.get_macro("PERSIST").unwrap(), Some("true".to_string()));
    }
}
