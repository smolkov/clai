/// Shipping in cents: 490 up to 2 kg, then 150 per started kg.
pub fn cost(weight_g: u64) -> u64 {
    if weight_g <= 2000 {
        490
    } else {
        490 + (weight_g - 2000).div_ceil(1000) * 150
    }
}
