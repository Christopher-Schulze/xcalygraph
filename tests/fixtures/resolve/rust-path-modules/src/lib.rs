#[path = "custom\u{5f}widget.rs"]
mod inventory;

#[path = "missing/custom.rs"]
mod missing;

#[path = "../../outside.rs"]
mod outside;

#[path = "thread_files"]
mod thread {
    #[path = r#"tls.rs"#]
    mod local_data;

    pub fn session() -> local_data::Session {
        local_data::Session::new()
    }
}

pub fn widget() -> inventory::Widget {
    inventory::Widget::new()
}

pub fn misleading_inventory() -> Option<inventory::MisleadingInventory> {
    None
}

pub fn missing_lookup() -> Option<missing::MisleadingFallback> {
    None
}

pub fn outside_lookup() -> Option<outside::MisleadingOutside> {
    None
}

#[path = "quoted\'widget.rs"]
mod quoted;

pub fn quoted_widget() -> quoted::ApostropheWidget {
    quoted::ApostropheWidget
}
