//! Integer math used by settlement. Kept separate so it can be tested on its own.

/// Splits `total` into parts proportional to `weights` using the largest remainder method,
/// so the parts always add up to exactly `total`. Ties go to the earlier index, which makes
/// the result deterministic for a fixed member order.
///
/// Returns all zeros if the weights sum to zero.
pub fn apportion(total: u128, weights: &[u128]) -> Vec<u128> {
    let sum: u128 = weights.iter().sum();
    if sum == 0 || weights.is_empty() {
        return vec![0; weights.len()];
    }
    let mut parts = Vec::with_capacity(weights.len());
    let mut rems = Vec::with_capacity(weights.len());
    let mut assigned: u128 = 0;
    for (i, w) in weights.iter().enumerate() {
        // total * w can't overflow for this contract's ranges (total < 2^64, w < 2^64),
        // and overflow-checks are on in release, so a bad input aborts instead of wrapping.
        let num = total * w;
        let part = num / sum;
        parts.push(part);
        rems.push((num % sum, i));
        assigned += part;
    }
    let mut left = total - assigned;
    // Largest remainder first, earlier index wins ties.
    rems.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, i) in rems.iter() {
        if left == 0 {
            break;
        }
        parts[*i] += 1;
        left -= 1;
    }
    parts
}

/// Coverage in basis points (10,000 = 100%). Zero demand counts as full coverage.
pub fn coverage_bps(collected: u128, demand: u128) -> u32 {
    if demand == 0 {
        return 10_000;
    }
    ((collected * 10_000) / demand) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apportion_sums_exactly() {
        let w = [1u128, 2, 3, 7, 8];
        for total in [0u128, 1, 7, 100, 1_000_003, 1_500_000_000] {
            let p = apportion(total, &w);
            assert_eq!(p.iter().sum::<u128>(), total, "total {total}");
        }
    }

    #[test]
    fn apportion_is_proportional_when_divisible() {
        assert_eq!(apportion(600, &[1, 2, 3]), vec![100, 200, 300]);
    }

    #[test]
    fn apportion_zero_weights() {
        assert_eq!(apportion(50, &[0, 0]), vec![0, 0]);
        assert!(apportion(50, &[]).is_empty());
    }

    #[test]
    fn apportion_remainder_goes_to_largest_fraction() {
        // 10 split 1:1:1 -> 3.33 each, one extra unit to the first (tie, earlier index).
        assert_eq!(apportion(10, &[1, 1, 1]), vec![4, 3, 3]);
    }

    #[test]
    fn coverage_math() {
        assert_eq!(coverage_bps(0, 0), 10_000);
        assert_eq!(coverage_bps(50, 100), 5_000);
        assert_eq!(coverage_bps(100, 100), 10_000);
        assert_eq!(coverage_bps(0, 100), 0);
    }
}
