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
    use tempfile::NamedTempFile;

    #[test]
    fn test_check_include() {
        let temp_file = NamedTempFile::new().unwrap();
        let db = OpticDb::new(temp_file.path().to_str().unwrap()).unwrap();

        let hash = [1u8; 32];

        // Initially, the hash should not be included
        let result = db.check_include(&hash).unwrap();
        assert_eq!(result, None);

        // Record the include
        db.record_include(&hash, 42).unwrap();

        // Now, the hash should be included
        let result = db.check_include(&hash).unwrap();
        assert_eq!(result, Some(42));
    }
}
