use core::sync::atomic::{AtomicU8, Ordering};

static RESPAWN: AtomicU8 = AtomicU8::new(1);
static EXP: AtomicU8 = AtomicU8::new(1);
pub fn respawn_rate() -> u8 {
    RESPAWN.load(Ordering::Relaxed)
}
pub fn set_respawn_rate(value: u8) {
    RESPAWN.store(value.clamp(1, 6), Ordering::Relaxed);
}
pub fn exp_rate() -> u8 {
    EXP.load(Ordering::Relaxed)
}
pub fn set_exp_rate(value: u8) {
    EXP.store(value.clamp(1, 6), Ordering::Relaxed);
}

pub fn respawn_counter(timer: u16, threshold: i16, rate: u8) -> u16 {
    if timer == u16::MAX || threshold <= 0 || timer >= threshold as u16 {
        return timer;
    }
    timer.saturating_add(u16::from(rate.clamp(1, 6)) - 1).min(threshold as u16)
}

pub fn kill_exp(base: i16, bonus: i16, rate: u8) -> i64 {
    (i64::from(base) + i64::from(bonus)) * i64::from(rate.clamp(1, 6))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn respawn_never_skips_equality_or_arms_a_disabled_timer() {
        for rate in 1..=6 {
            assert_eq!(respawn_counter(65535, 300, rate), 65535);
            let mut timer = 0;
            let mut ticks = 0;
            loop {
                let seen = respawn_counter(timer, 300, rate);
                ticks += 1;
                if seen == 300 {
                    break;
                }
                timer = seen + 1;
                assert!(ticks <= 301);
            }
            assert_eq!(ticks, 301u16.div_ceil(u16::from(rate)));
        }
    }
    #[test]
    fn exp_scales_the_entire_award_without_a_divisor_approximation() {
        for rate in 1..=6 {
            assert_eq!(kill_exp(101, 7, rate), 108 * i64::from(rate));
        }
        assert_eq!(kill_exp(30000, 3000, 6), 198000);
        assert_eq!(kill_exp(101, 7, 1), 108);
    }
}
