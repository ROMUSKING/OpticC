use redb::{Database, TableDefinition, ReadableDatabase};

const INCLUDES_TABLE: TableDefinition<&[u8; 32], u32> = TableDefinition::new("includes");
const SYMBOLS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("symbols");

pub struct DB {
    db: Database,
}

impl DB {
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

    pub fn get_file_hash(&self, key: &[u8; 32]) -> Result<Option<u32>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(INCLUDES_TABLE)?;
        if let Some(val) = table.get(key)? {
            Ok(Some(val.value()))
        } else {
            Ok(None)
        }
    }

    pub fn insert_file_hash(&self, key: &[u8; 32], offset: u32) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(INCLUDES_TABLE)?;
            table.insert(key, offset)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_macro_def(&self, key: &str) -> Result<Option<String>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SYMBOLS_TABLE)?;
        if let Some(val) = table.get(key)? {
            Ok(Some(val.value().to_string()))
        } else {
            Ok(None)
        }
    }

    pub fn insert_macro_def(&self, key: &str, value: &str) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SYMBOLS_TABLE)?;
            table.insert(key, value)?;
        }
        write_txn.commit()?;
        Ok(())
    }
}
