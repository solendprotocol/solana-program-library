use crate::state::{pack_decimal, unpack_decimal};
use solana_program::program_pack::IsInitialized;
use solana_program::{program_error::ProgramError, slot_history::Slot};

use crate::{
    error::LendingError,
    math::{Decimal, TryAdd, TryDiv, TryMul, TrySub},
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::program_pack::{Pack, Sealed};

/// Sliding Window Rate limiter
/// guarantee: at any point, the outflow between [cur_slot - slot.window_duration, cur_slot]
/// is less than 2x max_outflow.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimiter {
    /// window duration in slots
    pub window_duration: Slot,

    /// max outflow per window duration
    pub max_outflow: Decimal,

    // state
    prev_window: Window,
    cur_window: Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    slot_start: u64,
    qty: Decimal,
}

impl RateLimiter {
    /// initialize rate limiter
    pub fn new(window_duration: u64, max_outflow: Decimal, cur_slot: u64) -> Self {
        let slot_start = cur_slot / window_duration * window_duration;
        Self {
            max_outflow,
            window_duration,
            prev_window: Window {
                slot_start: slot_start - 1,
                qty: Decimal::zero(),
            },
            cur_window: Window {
                slot_start,
                qty: Decimal::zero(),
            },
        }
    }

    /// update rate limiter with new quantity. errors if rate limit has been reached
    pub fn update(&mut self, cur_slot: u64, qty: Decimal) -> Result<(), ProgramError> {
        assert!(cur_slot >= self.cur_window.slot_start);

        // floor wrt window duration
        let slot_start = cur_slot / self.window_duration * self.window_duration;

        // update prev window, current window
        match slot_start.cmp(&(self.cur_window.slot_start + self.window_duration)) {
            // |<-prev window->|<-cur window (cur_slot is in here)->|
            std::cmp::Ordering::Less => (),

            // |<-prev window->|<-cur window->| (cur_slot is in here) |
            std::cmp::Ordering::Equal => {
                self.prev_window = self.cur_window;
                self.cur_window = Window {
                    slot_start,
                    qty: Decimal::zero(),
                };
            }

            // |<-prev window->|<-cur window->|<-cur window + 1->| ... | (cur_slot is in here) |
            std::cmp::Ordering::Greater => {
                self.prev_window = Window {
                    slot_start: self.cur_window.slot_start - 1,
                    qty: Decimal::zero(),
                };
                self.cur_window = Window {
                    slot_start,
                    qty: Decimal::zero(),
                };
            }
        };

        // assume the prev_window's outflow is even distributed across the window
        // this isn't true, but it's a good enough approximation
        let prev_weight = Decimal::one().try_sub(
            Decimal::from(cur_slot - self.cur_window.slot_start + 1)
                .try_div(self.window_duration)?,
        )?;
        let cur_outflow = prev_weight
            .try_mul(self.prev_window.qty)?
            .try_add(self.cur_window.qty)?;

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
        let mut rate_limiter = RateLimiter::new(10, Decimal::from(100u64), 10);

        // case 1: no prev window, all quantity is taken up in first slot
        assert_eq!(
            rate_limiter.update(10, Decimal::from(101u64)),
            Err(LendingError::OutflowRateLimitExceeded.into())
        );
        assert_eq!(rate_limiter.update(10, Decimal::from(100u64)), Ok(()));
        for i in 11..20 {
            assert_eq!(
                rate_limiter.update(i, Decimal::from(1u64)),
                Err(LendingError::OutflowRateLimitExceeded.into())
            );
        }

        // case 2: prev window qty affects cur window's allowed qty. exactly 10 qty frees up every
        // slot.
        for i in 20..30 {
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

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(1, Decimal::from(u64::MAX), 1)
    }
}

impl Sealed for RateLimiter {}

impl IsInitialized for RateLimiter {
    fn is_initialized(&self) -> bool {
        true
    }
}

/// Size of RateLimiter when packed into account
pub const RATE_LIMITER_LEN: usize = 72;
impl Pack for RateLimiter {
    const LEN: usize = RATE_LIMITER_LEN;

    fn pack_into_slice(&self, dst: &mut [u8]) {
        let dst = array_mut_ref![dst, 0, RATE_LIMITER_LEN];
        let (
            max_outflow_dst,
            window_duration_dst,
            prev_window_slot_start_dst,
            prev_window_qty_dst,
            cur_window_slot_start_dst,
            cur_window_qty_dst,
        ) = mut_array_refs![dst, 16, 8, 8, 16, 8, 16];
        pack_decimal(self.max_outflow, max_outflow_dst);
        *window_duration_dst = self.window_duration.to_le_bytes();
        *prev_window_slot_start_dst = self.prev_window.slot_start.to_le_bytes();
        pack_decimal(self.prev_window.qty, prev_window_qty_dst);
        *cur_window_slot_start_dst = self.cur_window.slot_start.to_le_bytes();
        pack_decimal(self.cur_window.qty, cur_window_qty_dst);
    }

    fn unpack_from_slice(src: &[u8]) -> Result<Self, ProgramError> {
        let src = array_ref![src, 0, RATE_LIMITER_LEN];
        let (
            max_outflow_src,
            window_duration_src,
            prev_window_slot_start_src,
            prev_window_qty_src,
            cur_window_slot_start_src,
            cur_window_qty_src,
        ) = array_refs![src, 16, 8, 8, 16, 8, 16];

        Ok(Self {
            max_outflow: unpack_decimal(max_outflow_src),
            window_duration: u64::from_le_bytes(*window_duration_src),
            prev_window: Window {
                slot_start: u64::from_le_bytes(*prev_window_slot_start_src),
                qty: unpack_decimal(prev_window_qty_src),
            },
            cur_window: Window {
                slot_start: u64::from_le_bytes(*cur_window_slot_start_src),
                qty: unpack_decimal(cur_window_qty_src),
            },
        })
    }
}
