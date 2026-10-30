pub struct Disk {
    path: String,
}

impl Disk {
    pub fn mount(path: &str) -> Disk {
        Disk { path: path.to_owned() }
    }

    pub fn sync(&self) {}
}
