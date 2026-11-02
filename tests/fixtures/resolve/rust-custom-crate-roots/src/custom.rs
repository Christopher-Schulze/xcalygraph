#[path = "declared.rs"]
pub mod declared;

pub mod inline {
    #[path = "leaf.rs"]
    pub mod nested;
}

pub fn call() -> (declared::Declared, inline::nested::Nested) {
    (declared::Declared, inline::nested::Nested)
}
