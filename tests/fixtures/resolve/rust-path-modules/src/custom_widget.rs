#[path = "child.rs"]
mod details;

#[path = "nested_inline"]
mod inline {
    #[path = "leaf.rs"]
    mod leaf;

    pub fn value() -> leaf::PathLoadedInline {
        leaf::PathLoadedInline
    }
}

pub struct Widget;

impl Widget {
    pub fn new() -> Self {
        let _ = details::Component;
        Self
    }
}

pub fn misleading_nested() -> Option<details::MisleadingNested> {
    None
}
