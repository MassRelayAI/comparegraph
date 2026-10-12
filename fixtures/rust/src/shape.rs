pub trait Area {
    fn area(&self) -> f64;
}

pub struct Square {
    pub side: f64,
}

impl Area for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}
