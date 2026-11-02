trait Readable {
    fn value(&self) -> usize;
}

trait Unrelated {
    fn value(&self) -> usize;
}

struct Other {
    _value: usize,
}

impl Unrelated for Other {
    fn value(&self) -> usize {
        self._value
    }
}

pub fn bounded<T: Readable>(value: T) -> usize {
    let operation = || value.value();
    operation()
}

pub fn shadowed<T: Readable>(value: T) -> usize {
    let value = Other { _value: 0 };
    let operation = || value.value();
    operation()
}

pub fn unbounded<T>(value: T) -> usize {
    let operation = || value.value();
    operation()
}

pub fn where_bounded<T>(value: T) -> usize
where
    T: Readable,
{
    let operation = || value.value();
    operation()
}

struct T {
    _value: usize,
}

impl Unrelated for T {
    fn value(&self) -> usize {
        self._value
    }
}

struct Concrete {
    _value: usize,
}

impl Readable for Concrete {
    fn value(&self) -> usize {
        self._value
    }
}

struct Wrapper<T>(T);

impl<T: Readable> Wrapper<T> {
    fn read(value: T) -> usize {
        let operation = || value.value();
        operation()
    }
}

struct WhereWrapper<T>(T);

impl<T> WhereWrapper<T>
where
    T: Readable,
{
    fn read(value: T) -> usize {
        let operation = || value.value();
        operation()
    }
}
