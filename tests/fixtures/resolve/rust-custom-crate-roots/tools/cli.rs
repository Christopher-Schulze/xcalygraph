pub mod nested_group {
    #[path = "leaf.rs"]
    pub mod leaf;
}

pub fn tool_call() -> nested_group::leaf::CliNested {
    nested_group::leaf::CliNested
}
