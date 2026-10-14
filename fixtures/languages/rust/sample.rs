use std::collections::HashMap;

pub struct Counter {
    total: u32,
}

fn bump(n: u32) -> u32 {
    n + 1
}

impl Counter {
    pub fn add(&mut self, n: u32) -> u32 {
        self.total += bump(n);
        self.total
    }
}

pub fn run() -> usize {
    let mut c = Counter { total: 0 };
    c.add(2);
    let m: HashMap<u32, u32> = HashMap::new();
    m.len()
}
