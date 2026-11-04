struct Wrapper;
struct Value;
impl Wrapper {
    fn new() -> Wrapper { Wrapper }
    fn unwrap(self) -> Value { Value }
    fn finish(self) {}
}
impl Value {
    fn finish(self) {}
}
fn entry() { Wrapper::new().unwrap().finish(); }

struct Left;
struct Right;
impl Left { fn clash(self) {} } impl Right { fn clash(self) {} }
fn ambiguous_line() { Right.clash(); }

mod unrelated;
