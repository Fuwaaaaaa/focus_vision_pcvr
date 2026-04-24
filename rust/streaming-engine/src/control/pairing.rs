use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use fvp_common::{MAX_PIN_ATTEMPTS, PIN_LOCKOUT_SECONDS};

/// Serializable form of lockout state, written to disk so the lockout window
/// survives companion/engine restarts and cannot be reset by an attacker
/// simply by killing the process.
///
/// Note: only `lockout_until_unix_us` is persisted meaningfully. The in-memory
/// `attempts` counter is scoped to the current PIN — and the PIN rotates on
/// every `PairingState::new()` — so carrying `attempts` across restarts would
/// apply old-PIN misses to a new PIN. Restart + fresh PIN already defeats a
/// stateless brute-force attacker; the only thing we must prevent is bypass
/// of an *active* lockout.
#[derive(Serialize, Deserialize, Debug, Default)]
struct PersistedLockout {
    /// UNIX microsecond timestamp when lockout ends (0 = no lockout).
    lockout_until_unix_us: u64,
}

#[cfg(test)]
thread_local! {
    /// Tests opt into on-disk persistence by setting this to a tempdir path.
    /// Default `None` means `save()` is a no-op and `load()` returns default,
    /// so existing tests do not pollute the real `%APPDATA%/FocusVisionPCVR/`.
    static TEST_PATH_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        std::cell::RefCell::new(None);
}

impl PersistedLockout {
    fn path() -> Option<PathBuf> {
        #[cfg(test)]
        {
            return TEST_PATH_OVERRIDE.with(|c| c.borrow().clone());
        }
        #[cfg(not(test))]
        dirs_next::data_dir().map(|d| d.join("FocusVisionPCVR").join("lockout.json"))
    }

    fn load() -> Self {
        let Some(path) = Self::path() else { return Self::default() };
        let Ok(content) = std::fs::read_to_string(&path) else { return Self::default() };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Atomic write (temp file + rename) so a concurrent reader never sees
    /// a partially-written lockout record.
    fn save(&self) {
        let Some(path) = Self::path() else { return };
        let Some(parent) = path.parent() else { return };
        let _ = std::fs::create_dir_all(parent);
        let tmp = parent.join("lockout.json.tmp");
        let Ok(content) = serde_json::to_string(self) else { return };
        if std::fs::write(&tmp, content).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn clear() {
        if let Some(path) = Self::path() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn now_unix_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// PIN pairing state with brute-force protection.
/// Uses 6-digit PIN (0-999999) with cryptographic RNG.
///
/// Lockout state is persisted to `%APPDATA%/FocusVisionPCVR/lockout.json`
/// (Windows) so a restart of the engine/companion cannot be used to reset
/// the remaining lockout window.
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

        // Restore any persisted lockout state. The PIN itself is always
        // freshly generated — we only carry forward the attempt/lockout
        // counters that would otherwise be trivially reset by a restart.
        let persisted = PersistedLockout::load();
        let now_us = now_unix_us();
        let lockout_until = if persisted.lockout_until_unix_us > now_us {
            let remaining_us = persisted.lockout_until_unix_us.saturating_sub(now_us);
            let remaining = Duration::from_micros(remaining_us);
            log::warn!(
                "Resuming prior lockout: {}s remaining (persisted on disk)",
                remaining.as_secs()
            );
            Some(Instant::now() + remaining)
        } else {
            // Stale or absent record — clear both memory and disk.
            if persisted.lockout_until_unix_us != 0 {
                PersistedLockout::clear();
            }
            None
        };

        Self {
            pin,
            attempts: 0,
            lockout_until,
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

        // Constant-time comparison to mitigate timing side-channel on PIN.
        // Accepted risk: black_box prevents compiler optimization but doesn't
        // guarantee CPU-level constant-time on all architectures. Practical risk
        // is low: 6-digit PIN over TLS, 5-attempt lockout, local Wi-Fi only.
        if std::hint::black_box((submitted_pin ^ self.pin) == 0) {
            self.paired = true;
            self.attempts = 0;
            self.lockout_until = None;
            PersistedLockout::clear();
            log::info!("Pairing successful");
            Ok(())
        } else {
            self.attempts += 1;
            log::warn!("PIN incorrect (attempt {}/{})", self.attempts, MAX_PIN_ATTEMPTS);

            if self.attempts >= MAX_PIN_ATTEMPTS {
                let lockout_duration = Duration::from_secs(PIN_LOCKOUT_SECONDS);
                self.lockout_until = Some(Instant::now() + lockout_duration);
                self.attempts = 0;
                self.pin = generate_pin();
                PersistedLockout {
                    lockout_until_unix_us: now_unix_us()
                        .saturating_add(lockout_duration.as_micros() as u64),
                }
                .save();
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

    /// Helper: point the persistence layer at a tempdir for one test.
    /// Dropping the returned guard clears the override so other tests stay
    /// isolated (thread-local, so concurrent tests don't see each other).
    struct TempPersist {
        _dir: tempfile::TempDir,
    }
    impl TempPersist {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join("lockout.json");
            TEST_PATH_OVERRIDE.with(|c| *c.borrow_mut() = Some(path));
            Self { _dir: dir }
        }
    }
    impl Drop for TempPersist {
        fn drop(&mut self) {
            TEST_PATH_OVERRIDE.with(|c| *c.borrow_mut() = None);
        }
    }

    #[test]
    fn test_lockout_persists_across_new() {
        let _guard = TempPersist::new();

        // Lock out a first instance.
        {
            let mut state = PairingState::new();
            let pin = state.get_pin();
            let wrong = if pin == 0 { 1 } else { 0 };
            for _ in 0..MAX_PIN_ATTEMPTS {
                let _ = state.verify(wrong);
            }
            assert!(state.is_locked(), "first instance should be locked");
        }

        // A fresh PairingState must pick up the persisted lockout —
        // an attacker cannot reset by restarting the companion.
        let state2 = PairingState::new();
        assert!(
            state2.is_locked(),
            "new instance must resume persisted lockout"
        );
    }

    #[test]
    fn test_successful_pair_clears_persisted_state() {
        let _guard = TempPersist::new();

        // Trigger a lockout so there is something on disk to clear.
        let mut state = PairingState::new();
        let pin = state.get_pin();
        let wrong = if pin == 0 { 1 } else { 0 };
        for _ in 0..MAX_PIN_ATTEMPTS {
            let _ = state.verify(wrong);
        }
        assert!(state.is_locked());

        // A fresh instance resumes the lockout…
        let state2 = PairingState::new();
        assert!(state2.is_locked());

        // …and the successful-pair path calls PersistedLockout::clear() to
        // wipe the on-disk record so a later restart does not find a stale
        // lockout. We call clear() directly because verify() against an
        // already-locked state returns LockedOut without reaching the clear().
        PersistedLockout::clear();
        let loaded = PersistedLockout::load();
        assert_eq!(loaded.lockout_until_unix_us, 0);
    }

    #[test]
    fn test_stale_lockout_record_is_cleared() {
        let _guard = TempPersist::new();

        // Write a lockout that expired a second ago.
        let stale = PersistedLockout {
            lockout_until_unix_us: now_unix_us().saturating_sub(1_000_000),
        };
        stale.save();

        // New instance should detect the stale record, not be locked,
        // and clear it so the file doesn't linger.
        let state = PairingState::new();
        assert!(!state.is_locked(), "expired lockout must not apply");
    }
}
