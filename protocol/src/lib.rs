pub fn placeholder() -> u32 {
    42
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_expected_value() {
        assert_eq!(placeholder(), 42);
    }
}
