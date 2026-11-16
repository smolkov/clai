use crate::shipping;

/// The total of an order: items, then shipping by weight.
pub fn total(items: &[(u64, u32)], weight_g: u64) -> u64 {
    let items: u64 = items.iter().map(|(price, n)| price * u64::from(*n)).sum();
    items + shipping::cost(weight_g)
}
