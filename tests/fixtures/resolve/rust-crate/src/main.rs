mod convert;
mod model;
mod store;

use store::{Reader, Store};

fn main() {
    let store = Store::open("x");
    let count = store::load(&store);
    report(count, &store);
}

fn report(count: usize, reader: &dyn Reader) {
    println!("{count} {}", reader.len());
}
