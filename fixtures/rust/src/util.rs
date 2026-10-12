use crate::shape::{Area, Square};

pub fn double(x: f64) -> f64 {
    x * 2.0
}

pub fn total(s: &Square) -> f64 {
    double(s.area())
}
