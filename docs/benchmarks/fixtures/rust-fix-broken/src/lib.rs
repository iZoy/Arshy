pub fn add(a: i32, b: i32) -> i32 {
    a - b // wrong
}

pub fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        hi // wrong branch
    } else if v > hi {
        lo // wrong branch
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_numbers() {
        assert_eq!(add(2, 3), 5);
    }

    #[test]
    fn clamps_low() {
        assert_eq!(clamp(-1, 0, 10), 0);
    }

    #[test]
    fn clamps_high() {
        assert_eq!(clamp(99, 0, 10), 10);
    }
}
