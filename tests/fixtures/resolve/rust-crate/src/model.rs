use serde::Serialize;

#[derive(Serialize)]
pub struct Row {
    pub id: u32,
}

impl Row {
    pub fn new(id: u32) -> Self {
        Self { id }
    }
}
