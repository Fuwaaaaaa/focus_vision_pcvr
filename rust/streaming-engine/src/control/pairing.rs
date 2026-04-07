use std::time::{Duration, Instant};
use fvp_common::{MAX_PIN_ATTEMPTS, PIN_LOCKOUT_SECONDS};

/// PIN pairing state with brute-force protection.
/// Uses 6-digit PIN (0-999999) with cryptographic RNG.
pub struct PairingState {
    pin: u32,
    attempts: u8,
    lockout_until: Option<Instant>,
    paired: bool,
}

impl Default for PairingState {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingState {
    pub fn new() -> Self {
        let pin = generate_pin();
        log::info!("Pairing PIN: {:06}", pin);
        Self {
            pin,
            attempts: 0,
            lockout_until: None,
            paired: false,
        }
    }

    /// Check if currently locked out.
    pub fn is_locked(&self) -> bool {
        if let Some(until) = self.lockout_until {
            Instant::now() < until
        } else {
            false
        }
    }

    /// Attempt to verify a PIN. Returns Ok(()) on success, Err(reason) on failure.
    pub fn verify(&mut self, submitted_pin: u32) -> Result<(), PairingError> {
        if self.paired {
            return Ok(());
        }
        if self.is_locked() {
            return Err(PairingError::LockedOut);
        }

        if submitted_pin == self.pin {
            self.paired = true;
            self.attempts = 0;
            self.lockout_until = None;
            log::info!("Pairing successful");
            Ok(())
        } else {
            self.attempts += 1;
            log::warn!("PIN incorrect (attempt {}/{})", self.attempts, MAX_PIN_ATTEMPTS);

            if self.attempts >= MAX_PIN_ATTEMPTS {
                self.lockout_until = Some(
                    Instant::now() + Duration::from_secs(PIN_LOCKOUT_SECONDS),
                );
                self.attempts = 0;
                self.pin = generate_pin();
                log::warn!("Locked out for {}s. New PIN: {:06}", PIN_LOCKOUT_SECONDS, self.pin);
                Err(PairingError::LockedOut)
            } else {
                Err(PairingError::WrongPin {
                    remaining: MAX_PIN_ATTEMPTS - self.attempts,
                })
            }
        }
    }

    pub fn is_paired(&self) -> bool {
        self.paired
    }

    pub fn get_pin(&self) -> u32 {
        self.pin
    }
}

#[derive(Debug)]
pub enum PairingError {
    WrongPin { remaining: u8 },
    LockedOut,
}

fn generate_pin() -> u32 {
    rand::random::<u32>() % 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_correct_pin() {
        let mut state = PairingState::new();
        let pin = state.get_pin();
        assert!(state.verify(pin).is_ok());
        assert!(state.is_paired());
    }

    #[test]
    fn test_wrong_pin() {
        let mut state = PairingState::new();
        let pin = state.get_pin();
        let wrong = if pin == 0 { 1 } else { 0 };
        let result = state.verify(wrong);
        assert!(result.is_err());
        assert!(!state.is_paired());
    }

    #[test]
    fn test_lockout_after_max_attempts() {
        let mut state = PairingState::new();
        let wrong = 999_999u32.wrapping_add(1);
        for _ in 0..MAX_PIN_ATTEMPTS {
            let _ = state.verify(wrong);
        }
        assert!(state.is_locked());
        let pin = state.get_pin();
        assert!(matches!(state.verify(pin), Err(PairingError::LockedOut)));
    }

    #[test]
    fn test_pin_range_is_6_digits() {
        for _ in 0..100 {
            let pin = generate_pin();
            assert!(pin < 1_000_000, "PIN {} exceeds 6-digit range", pin);
        }
    }

    #[test]
    fn test_pin_not_always_same() {
        // Generate 10 PINs; at least 2 should differ (cryptographic RNG)
        let pins: Vec<u32> = (0..10).map(|_| generate_pin()).collect();
        let unique: std::collections::HashSet<u32> = pins.into_iter().collect();
        assert!(unique.len() >= 2, "All PINs identical — RNG may be broken");
    }
}
