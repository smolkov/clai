/// Sums of each window of `size` neighbouring values.
pub fn window_sums(values: &[i64], size: usize) -> Vec<i64> {
    if size == 0 || size > values.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for start in 0..values.len() - size {
        out.push(values[start..start + size].iter().sum());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows() {
        assert_eq!(window_sums(&[1, 2, 3, 4], 2), [3, 5, 7]);
        assert_eq!(window_sums(&[5], 1), [5]);
        assert_eq!(window_sums(&[1, 2], 3), Vec::<i64>::new());
    }
}
