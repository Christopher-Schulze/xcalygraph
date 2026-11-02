#[path = "shared.rs"]
pub mod first;
#[path = "shared.rs"]
pub mod second;

pub fn construct() -> first::Thing {
    first::Thing
}
