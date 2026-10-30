//! Row storage.

use std::collections::HashMap;
use std::env;

pub use crate::model::{Row, RowId as Id};
use super::db::*;

/// Maximum number of rows.
pub const MAX_ROWS: usize = 100;

static NAME: &str = "rows";

/// Reads rows. More text.
pub trait Reader: Send {
    /// Returns one row.
    fn get(&self, id: Id) -> Option<&Row>;
    fn len(&self) -> usize;
}

/// Holds rows in memory.
#[derive(Debug, Default)]
pub struct Store {
    rows: HashMap<Id, Row>,
    client: Client,
}

pub enum Mode {
    Read,
    Write,
}

pub type Rows = Vec<Row>;

macro_rules! row {
    ($id:expr) => {
        Row::new($id)
    };
}

impl Store {
    /// Opens a store.
    pub fn open(path: &str) -> Result<Self, Error> {
        let client = Client::connect(path)?;
        let url = env::var("DB_URL");
        let fallback = option_env!("BUILD_TAG");
        Ok(Self { rows: HashMap::new(), client })
    }

    fn insert(&mut self, row: Row) {
        self.client.log(&row);
        self.rows.insert(row.id, row);
    }
}

impl Reader for Store {
    fn get(&self, id: Id) -> Option<&Row> {
        self.rows.get(&id)
    }

    fn len(&self) -> usize {
        self.rows.len()
    }
}

pub(crate) fn load(reader: &dyn Reader) -> usize {
    let store = Store::open("x").unwrap_or_default();
    let other: Store = Store::default();
    let row = row!(1);
    helpers::check(&row);
    reader.len() + store.len() + other.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens() {
        let store = Store::open("x");
        assert!(store.is_ok());
    }
}
