struct Local {
    exported: bool,
}

impl Local {
    fn exported(&self) -> bool {
        self.exported
    }

    fn peek(&self) {}
}

fn calls_iterator_method() {
    let mut chars = "x".chars().peekable();
    chars.peek();
}

fn formats_field() {
    let symbol = Local { exported: true };
    let _ = format!("{}", if symbol.exported { "pub" } else { "priv" });
}

fn parse(candidate: &str) -> usize {
    candidate.len()
}

fn closure_shadows_parse(candidate: &str) {
    let parse = |version: &str| version.len();
    parse(candidate);
}

fn block_closure_restores_parse(candidate: &str) {
    {
        let parse = |version: &str| version.len();
        parse(candidate);
    }
    parse(candidate);
}
