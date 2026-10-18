//! Pure geometry for the visuals (spec §3.8): the tree in, shapes out, no window needed.

pub mod sunburst;
pub mod treemap;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn area(&self) -> f32 {
        self.w * self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// Whether `other` lies inside this rectangle, give or take rounding.
    pub fn encloses(&self, other: &Rect) -> bool {
        const E: f32 = 0.01;
        other.x >= self.x - E
            && other.y >= self.y - E
            && other.x + other.w <= self.x + self.w + E
            && other.y + other.h <= self.y + self.h + E
    }
}
