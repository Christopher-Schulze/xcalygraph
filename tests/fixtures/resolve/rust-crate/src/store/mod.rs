mod disk;

use crate::model::Row;

pub trait Reader {
    fn len(&self) -> usize;
    fn first(&self) -> Option<&Row>;
}

pub struct Store {
    rows: Vec<Row>,
    disk: disk::Disk,
}

impl Store {
    pub fn open(path: &str) -> Self {
        let disk = disk::Disk::mount(path);
        let row = Row::new(1);
        Store { rows: vec![row], disk }
    }

    pub fn flush_all(&self) {
        self.disk.sync();
    }
}

impl Reader for Store {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn first(&self) -> Option<&Row> {
        self.flush_all();
        self.rows.first()
    }
}

pub fn load(store: &Store) -> usize {
    store.flush_all();
    store.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads() {
        let store = Store::open("t");
        assert_eq!(load(&store), 1);
    }
}
