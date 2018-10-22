use std::ops::Not;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Not for Side {
    type Output = Side;

    fn not(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub side: Side,
    pub count: usize,
}

impl State {
    #[cfg(target_pointer_width = "64")]
    const MASK: usize = 0x7FFFFFFFFFFFFFFF;

    #[cfg(target_pointer_width = "32")]
    const MASK: usize = 0x7FFFFFFF;

    fn from_usize(v: usize) -> Self {
        let side = match v & !Self::MASK {
            0 => Side::Left,
            _ => Side::Right,
        };

        let count = v & Self::MASK;

        Self { side, count }
    }

    fn to_usize(self) -> usize {
        let mut value = self.count;
        if self.side == Side::Right {
            value |= !Self::MASK;
        }
        value
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct ToggleCount(AtomicUsize);

impl ToggleCount {
    pub fn load(&self, order: Ordering) -> State {
        State::from_usize(self.0.load(order))
    }

    pub fn store(&self, state: State, order: Ordering) {
        let value = state.to_usize();
        self.0.store(value, order);
    }

    pub fn compare_and_swap(&self, current: State, new: State, order: Ordering) -> State {
        let current = current.to_usize();
        let new = new.to_usize();

        State::from_usize(self.0.compare_and_swap(current, new, order))
    }

    pub fn swap(&self, val: State, order: Ordering) -> State {
        let val = val.to_usize();
        State::from_usize(self.0.swap(val, order))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_to_usize() {
        let state = State {
            side: Side::Left,
            count: 1,
        };

        let actual = state.to_usize();
        let expected = 1;

        assert_eq!(expected, actual);
    }

    #[test]
    fn state_from_usize_right() {
        let v = usize::max_value();

        let actual = State::from_usize(v);
        assert_eq!(Side::Right, actual.side);
        assert_eq!(v & State::MASK, actual.count);
    }

    #[test]
    fn state_from_usize_left() {
        let v = 0;

        let actual = State::from_usize(v);
        assert_eq!(Side::Left, actual.side);
        assert_eq!(0, actual.count);
    }
}
