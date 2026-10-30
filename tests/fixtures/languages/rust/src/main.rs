mod store;

fn main() {
    let count = store::load(&store::Store::default());
    println!("{count}");
}
