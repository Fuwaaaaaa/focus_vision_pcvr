//! The engine's log file, in a process of its own: a process has one
//! logger, and this test installs it.

#[test]
fn the_engine_log_lands_in_its_file() {
    // REGRESSION: the engine logged to vrserver.exe's stderr, which is kept
    // nowhere, and without RUST_LOG only errors.
    if std::env::var_os("RUST_LOG").is_some() {
        eprintln!("RUST_LOG is set; the default filter is what this checks");
        return;
    }
    let dir = std::env::temp_dir().join(format!("fvp-engine-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    streaming_engine::logging::init(Some(&dir));

    log::info!(target: "streaming_engine::engine", "engine info reaches the file");
    log::debug!(target: "streaming_engine::engine", "engine debug stays out");
    log::warn!(target: "rustls::conn", "a dependency's warning reaches the file");
    log::info!(target: "rustls::conn", "a dependency's info stays out");

    let text = std::fs::read_to_string(dir.join("engine.log")).expect("engine.log");
    assert!(text.contains("engine info reaches the file"), "{text}");
    assert!(text.contains("a dependency's warning reaches the file"), "{text}");
    assert!(!text.contains("stays out"), "{text}");
    assert!(text.contains(" INFO "), "records carry their level: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}
