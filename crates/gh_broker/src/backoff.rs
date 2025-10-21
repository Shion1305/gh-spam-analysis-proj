use std::time::Duration;

pub fn exponential_jitter_backoff(
    base: Duration,
    attempt: u32,
    max: Duration,
    jitter_frac: f32,
) -> Duration {
    let capped_attempt = attempt.min(8);
    let factor = 1u64.checked_shl(capped_attempt).unwrap_or(1 << 8);
    let raw = base.saturating_mul(factor as u32);
    let capped = if raw > max { max } else { raw };
    let nanos = capped.as_nanos() as i128;
    let jitter = ((nanos as f64) * (jitter_frac as f64)).round() as i128;
    let delta = fastrand::i128(-jitter..=jitter);
    let result = (nanos + delta).max(0) as u128;
    Duration::from_nanos(result as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_increases_and_caps() {
        let base = Duration::from_millis(200);
        let max = Duration::from_secs(5);
        let a1 = exponential_jitter_backoff(base, 1, max, 0.0);
        let a4 = exponential_jitter_backoff(base, 4, max, 0.0);
        assert!(a4 >= a1);
        assert!(a4 <= max);
    }
}
