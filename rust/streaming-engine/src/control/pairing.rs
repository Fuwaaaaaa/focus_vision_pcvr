use std::time::{Duration, Instant};
use fvp_common::{MAX_PIN_ATTEMPTS, PIN_LOCKOUT_SECONDS};

/// PIN pairing state with brute-force protection.
pub struct PairingState {
    pin: u16,
    attempts: u8,
    lockout_until: Option<Instant>,
    paired: bool,
}

impl PairingState {
    pub fn new() -> Self {
        let pin = generate_pin();
        log::info!("Pairing PIN: {:04}", pin);
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
    pub fn verify(&mut self, submitted_pin: u16) -> Result<(), PairingError> {
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
                // Generate new PIN after lockout
                self.pin = generate_pin();
                log::warn!("Locked out for {}s. New PIN: {:04}", PIN_LOCKOUT_SECONDS, self.pin);
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

    pub fn get_pin(&self) -> u16 {
        self.pin
    }
}

#[derive(Debug)]
pub enum PairingError {
    WrongPin { remaining: u8 },
    LockedOut,
}

fn generate_pin() -> u16 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (seed % 10000) as u16
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
        let wrong = 9999u16.wrapping_add(1); // always 0 or different
        for _ in 0..MAX_PIN_ATTEMPTS {
            let _ = state.verify(wrong);
        }
        assert!(state.is_locked());
        // Even correct PIN fails during lockout
        let pin = state.get_pin();
        assert!(matches!(state.verify(pin), Err(PairingError::LockedOut)));
    }
}
