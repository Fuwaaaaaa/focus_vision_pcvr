#![no_main]

use libfuzzer_sys::fuzz_target;
use streaming_engine::config::AppConfig;

fuzz_target!(|data: &[u8]| {
    // Try to interpret fuzzed bytes as UTF-8, then parse as TOML config
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // Parse TOML — must not panic on any input
    let mut config: AppConfig = match toml::from_str(text) {
        Ok(c) => c,
        Err(_) => return, // Invalid TOML is expected
    };

    // Validate — must not panic on any parsed config
    let _errors = config.validate();
});
