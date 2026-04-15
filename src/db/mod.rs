use redb::{Database, TableDefinition};

const INCLUDES_TABLE: TableDefinition<&[u8; 32], u32> = TableDefinition::new("includes");
const SYMBOLS_TABLE: TableDefinition<&str, u32> = TableDefinition::new("symbols");

pub struct OpticDb {
    db: Database,
}

impl OpticDb {
    pub fn new(path: &str) -> Result<Self, redb::Error> {
        let db = Database::create(path)?;
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(INCLUDES_TABLE)?;
            let _ = write_txn.open_table(SYMBOLS_TABLE)?;
        }
        write_txn.commit()?;
        Ok(Self { db })
    }

    pub fn check_include(&self, hash: &[u8; 32]) -> Result<Option<u32>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(INCLUDES_TABLE)?;
        if let Some(val) = table.get(hash)? {
            Ok(Some(val.value()))
        } else {
            Ok(None)
        }
    }

    pub fn record_include(&self, hash: &[u8; 32], offset: u32) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(INCLUDES_TABLE)?;
            table.insert(hash, offset)?;
        }
        write_txn.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_include() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("test_optic_db_{}.redb", timestamp));
        let path_str = path.to_str().unwrap();

        // Ensure clean state
        let _ = std::fs::remove_file(path_str);

        let db = OpticDb::new(path_str).expect("Failed to create OpticDb");
        let hash = [1u8; 32];
        let offset = 42;

        // Initially it should not be in the db
        let res = db.check_include(&hash).expect("Failed to check include");
        assert_eq!(res, None);

        // Record the include
        db.record_include(&hash, offset).expect("Failed to record include");

        // Now it should be present
        let res = db.check_include(&hash).expect("Failed to check include again");
        assert_eq!(res, Some(offset));

        // Another hash should still be missing
        let hash2 = [2u8; 32];
        let res2 = db.check_include(&hash2).expect("Failed to check include for hash2");
        assert_eq!(res2, None);

        // Drop the db to release file handles before cleanup
        drop(db);

        // Cleanup
        let _ = std::fs::remove_file(path_str);
    }
}
