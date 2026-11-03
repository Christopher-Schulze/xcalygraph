#[derive(Default)]
struct Widget;
impl Widget { fn make() -> Self { Widget } }
fn good() -> Widget { Widget }
fn trigger() {
    let _ = vec![Widget::default(), Widget::make(), good()];
}
