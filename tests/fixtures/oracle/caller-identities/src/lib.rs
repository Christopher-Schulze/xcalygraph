pub fn first() {}
pub fn second() {}
pub fn left() { first(); } pub fn right() { second(); }
pub fn outer() { fn nested() { first(); } nested(); }
