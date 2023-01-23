use solana_program::{program_error::ProgramError, slot_history::Slot};

use crate::{
    error::LendingError,
    math::{Decimal, TryAdd, TryDiv, TryMul, TrySub},
};

/// Sliding Window Rate limiter
/// guarantee: at any point, the outflow between [cur_slot - slot.window_duration, cur_slot]
/// is less than 2x max_outflow.

#[derive(Debug, Clone, Copy)]
pub struct RateLimiter {
    // parameters
    max_outflow: Decimal,
    window_duration: Slot,

    // state
    prev_window: Option<Window>,
    cur_window: Window,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    slot_start: u64,
    qty: Decimal,
}

impl RateLimiter {
    fn new(max_outflow: Decimal, window_duration: u64, cur_slot: u64) -> Self {
        Self {
            max_outflow,
            window_duration,
            prev_window: None,
            cur_window: Window {
                slot_start: cur_slot / window_duration * window_duration,
                qty: Decimal::zero(),
            },
        }
    }

    fn update(&mut self, cur_slot: u64, qty: Decimal) -> Result<(), ProgramError> {
        assert!(cur_slot >= self.cur_window.slot_start);

        // floor wrt window duration
        let slot_start = cur_slot / self.window_duration * self.window_duration;

        // update prev window, current window
        match slot_start.cmp(&(self.cur_window.slot_start + self.window_duration)) {
            // |<-prev window->|<-cur window (cur_slot is in here)->|
            std::cmp::Ordering::Less => (),

            // |<-prev window->|<-cur window->| (cur_slot is in here) |
            std::cmp::Ordering::Equal => {
                self.prev_window = Some(self.cur_window);
                self.cur_window = Window {
                    slot_start,
                    qty: Decimal::zero(),
                };
            }

            // |<-prev window->|<-cur window->|<-cur window + 1->| ... | (cur_slot is in here) |
            std::cmp::Ordering::Greater => {
                self.prev_window = None;
                self.cur_window = Window {
                    slot_start,
                    qty: Decimal::zero(),
                };
            }
        };

        let cur_outflow = match self.prev_window {
            None => self.cur_window.qty,
            Some(window) => {
                // assume the prev_window's outflow is even distributed across the window
                // this isn't true, but it's a good enough approximation
                let prev_weight = Decimal::one().try_sub(
                    Decimal::from(cur_slot - self.cur_window.slot_start + 1)
                        .try_div(self.window_duration)?,
                )?;

                (prev_weight.try_mul(window.qty)?).try_add(self.cur_window.qty)?
            }
        };

        if cur_outflow.try_add(qty)? > self.max_outflow {
            Err(LendingError::OutflowRateLimitExceeded.into())
        } else {
            self.cur_window.qty = self.cur_window.qty.try_add(qty)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_rate_limiter() {
        let mut rate_limiter = RateLimiter::new(Decimal::from(100u64), 10, 0);

        // case 1: no prev window, all quantity is taken up in first slot
        assert_eq!(rate_limiter.update(0, Decimal::from(100u64)), Ok(()));
        for i in 1..10 {
            assert_eq!(
                rate_limiter.update(i, Decimal::from(1u64)),
                Err(LendingError::OutflowRateLimitExceeded.into())
            );
        }

        // case 2: prev window qty affects cur window's allowed qty. exactly 10 qty frees up every
        // slot.
        for i in 10..20 {
            assert_eq!(
                rate_limiter.update(i, Decimal::from(11u64)),
                Err(LendingError::OutflowRateLimitExceeded.into())
            );

            assert_eq!(rate_limiter.update(i, Decimal::from(10u64)), Ok(()));

            assert_eq!(
                rate_limiter.update(i, Decimal::from(1u64)),
                Err(LendingError::OutflowRateLimitExceeded.into())
            );
        }

        // case 3: new slot is so far ahead, prev window is dropped
        assert_eq!(rate_limiter.update(100, Decimal::from(10u64)), Ok(()));
        for i in 101..109 {
            assert_eq!(rate_limiter.update(i, Decimal::from(10u64)), Ok(()));
        }
        println!("{:#?}", rate_limiter);
    }
}
