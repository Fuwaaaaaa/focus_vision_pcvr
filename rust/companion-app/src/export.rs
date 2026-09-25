use std::io::Write;
use std::path::PathBuf;

use crate::adb;

/// Sanitize PII from log text before it leaves the machine in a diagnostics
/// zip: Wi-Fi SSIDs, the pairing PIN, user names in profile paths, e-mail
/// addresses, MAC addresses and IPv4/IPv6 addresses.
///
/// Every scanner anchors its matches on ASCII bytes and copies everything
/// else through as `&str` slices, so non-ASCII text (Japanese log lines,
/// SSIDs, user names) survives byte-for-byte. Over-masking an IP-looking
/// token is preferred to leaking a real address.
pub(crate) fn sanitize_pii(text: &str) -> String {
    // Key/value maskers first: an SSID value may itself look like anything.
    let text = mask_ssids(text);
    let text = mask_pins(&text);
    let text = mask_user_paths(&text);
    let text = mask_emails(&text);
    // MAC before IPv6: six hex pairs would otherwise read as a colon-hex run.
    let text = mask_macs(&text);
    let text = mask_ipv4(&text);
    mask_ipv6(&text)
}

/// Replace `spans` (sorted, non-overlapping byte ranges) of `text` with
/// `label`. Callers only produce ranges whose ends sit on ASCII bytes or the
/// string ends, which are always char boundaries.
fn replace_spans(text: &str, spans: &[(usize, usize)], label: &str) -> String {
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for &(start, end) in spans {
        out.push_str(&text[last..start]);
        out.push_str(label);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Byte offsets where the value of `key` starts: `key` as a whole word
/// (ASCII case-insensitive), then an optional closing quote (JSON keys),
/// optional spaces, `:` or `=`, optional spaces.
fn key_value_starts(text: &str, key: &str) -> Vec<usize> {
    let b = text.as_bytes();
    let k = key.as_bytes();
    let mut starts = Vec::new();
    let mut i = 0;
    while i + k.len() <= b.len() {
        if b[i..i + k.len()].eq_ignore_ascii_case(k) && (i == 0 || !is_word_byte(b[i - 1])) {
            let mut j = i + k.len();
            if j < b.len() && b[j] == b'"' {
                j += 1;
            }
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && (b[j] == b':' || b[j] == b'=') {
                j += 1;
                while j < b.len() && b[j] == b' ' {
                    j += 1;
                }
                starts.push(j);
                i = j;
                continue;
            }
        }
        i += 1;
    }
    starts
}

/// `SSID: "name"`, `ssid=name`, `"SSID":"name"` → the value is replaced.
/// `BSSID` is skipped here (not a whole-word match); its value is a MAC.
fn mask_ssids(text: &str) -> String {
    let b = text.as_bytes();
    let mut spans = Vec::new();
    for start in key_value_starts(text, "ssid") {
        if start >= b.len() {
            continue;
        }
        if b[start] == b'"' {
            // Quoted: mask up to the closing quote, honouring \" escapes.
            let mut j = start + 1;
            while j < b.len() && b[j] != b'"' && b[j] != b'\n' {
                j += if b[j] == b'\\' && j + 1 < b.len() { 2 } else { 1 };
            }
            let end = j.min(b.len());
            if end > start + 1 {
                spans.push((start + 1, end));
            }
        } else if b[start] != b'<' {
            // Unquoted (`<unknown ssid>` is Android's placeholder, not PII).
            let mut j = start;
            while j < b.len() && !b[j].is_ascii_whitespace() && !b",;)}]".contains(&b[j]) {
                j += 1;
            }
            if j > start {
                spans.push((start, j));
            }
        }
    }
    replace_spans(text, &spans, "[REDACTED_SSID]")
}

/// `Pairing PIN: 123456`, `"pin":"123456"`, `pin=0421` → the digits are
/// replaced. `PIN_RESPONSE`, `PIN accepted`, the `------` sentinel are kept.
fn mask_pins(text: &str) -> String {
    let b = text.as_bytes();
    let mut spans = Vec::new();
    for start in key_value_starts(text, "pin") {
        let digits_start = if b.get(start) == Some(&b'"') { start + 1 } else { start };
        let mut j = digits_start;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let len = j - digits_start;
        if (4..=8).contains(&len) && (j == b.len() || !is_word_byte(b[j])) {
            spans.push((digits_start, j));
        }
    }
    replace_spans(text, &spans, "[REDACTED_PIN]")
}

/// `C:\Users\<name>\…`, `/Users/<name>/…`, `/home/<name>/…` (any case, also
/// JSON-escaped `\\`) → `<name>` is replaced.
fn mask_user_paths(text: &str) -> String {
    let b = text.as_bytes();
    let is_sep = |c: u8| c == b'\\' || c == b'/';
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !is_sep(b[i]) {
            i += 1;
            continue;
        }
        let marker_len = [&b"users"[..], &b"home"[..]].iter().find_map(|m| {
            let end = i + 1 + m.len();
            (end < b.len() && b[i + 1..end].eq_ignore_ascii_case(m) && is_sep(b[end]))
                .then_some(m.len())
        });
        let Some(marker_len) = marker_len else {
            i += 1;
            continue;
        };
        let mut name_start = i + 1 + marker_len;
        while name_start < b.len() && is_sep(b[name_start]) {
            name_start += 1;
        }
        // A user name may contain spaces, so it runs to the next separator;
        // if the path ends at the name itself, stop at whitespace instead.
        let mut j = name_start;
        while j < b.len() && !is_sep(b[j]) && !b"\"'\r\n\t<>|*?:,;".contains(&b[j]) {
            j += 1;
        }
        let name_end = if j < b.len() && is_sep(b[j]) {
            j
        } else {
            b[name_start..j]
                .iter()
                .position(|c| c.is_ascii_whitespace())
                .map_or(j, |p| name_start + p)
        };
        if name_end > name_start && !text[name_start..name_end].starts_with("[REDACTED") {
            spans.push((name_start, name_end));
        }
        i = name_end.max(i + 1);
    }
    replace_spans(text, &spans, "[REDACTED_USER]")
}

/// `local@example.com` → the whole address is replaced. A trailing
/// sentence period is not part of the domain.
fn mask_emails(text: &str) -> String {
    let b = text.as_bytes();
    let is_local = |c: u8| c.is_ascii_alphanumeric() || b"._%+-".contains(&c);
    let is_domain = |c: u8| c.is_ascii_alphanumeric() || c == b'.' || c == b'-';
    let mut spans = Vec::new();
    let mut last_end = 0;
    for at in (0..b.len()).filter(|&i| b[i] == b'@') {
        if at < last_end {
            continue;
        }
        let mut start = at;
        while start > last_end && is_local(b[start - 1]) {
            start -= 1;
        }
        while start < at && b[start] == b'.' {
            start += 1;
        }
        let mut end = at + 1;
        while end < b.len() && is_domain(b[end]) {
            end += 1;
        }
        while end > at + 1 && (b[end - 1] == b'.' || b[end - 1] == b'-') {
            end -= 1;
        }
        if start == at {
            continue;
        }
        let domain = &text[at + 1..end];
        let valid_tld = domain.rfind('.').is_some_and(|dot| {
            let tld = &domain[dot + 1..];
            dot > 0 && tld.len() >= 2 && tld.bytes().all(|c| c.is_ascii_alphabetic())
        });
        if valid_tld {
            spans.push((start, end));
            last_end = end;
        }
    }
    replace_spans(text, &spans, "[REDACTED_EMAIL]")
}

/// `aa:bb:cc:dd:ee:ff` / `AA-BB-CC-DD-EE-FF` (one separator style, not part
/// of a longer hex run) → replaced.
fn mask_macs(text: &str) -> String {
    const MAC_LEN: usize = 17;
    let b = text.as_bytes();
    let is_mac_at = |i: usize| {
        let sep = b[i + 2];
        (sep == b':' || sep == b'-')
            && (0..6).all(|g| b[i + g * 3].is_ascii_hexdigit() && b[i + g * 3 + 1].is_ascii_hexdigit())
            && (0..5).all(|g| b[i + g * 3 + 2] == sep)
    };
    let mut spans = Vec::new();
    let mut i = 0;
    while i + MAC_LEN <= b.len() {
        let end = i + MAC_LEN;
        if (i == 0 || !b[i - 1].is_ascii_hexdigit())
            && (end == b.len() || !b[end].is_ascii_hexdigit())
            && is_mac_at(i)
        {
            spans.push((i, end));
            i = end;
        } else {
            i += 1;
        }
    }
    replace_spans(text, &spans, "[REDACTED_MAC]")
}

/// Dotted-quad IPv4 with every octet 0–255. A run of digits and dots is only
/// masked when it holds exactly four numbers, so version strings such as
/// `10.0.19045.1234` (octet > 255) or `1.2.3.4.5` (five parts) are kept,
/// while a sentence-ending `192.168.1.5.` is caught. Ports stay visible.
fn mask_ipv4(text: &str) -> String {
    let b = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !(b[i].is_ascii_digit() || b[i] == b'.') {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
            i += 1;
        }
        let mut parts = Vec::new();
        let mut p = run_start;
        while p < i {
            if b[p] == b'.' {
                p += 1;
                continue;
            }
            let s = p;
            while p < i && b[p].is_ascii_digit() {
                p += 1;
            }
            parts.push((s, p));
        }
        let is_quad = parts.len() == 4
            && parts.windows(2).all(|w| w[1].0 == w[0].1 + 1)
            && parts.iter().all(|&(s, e)| {
                e - s <= 3 && text[s..e].parse::<u16>().is_ok_and(|v| v <= 255)
            });
        if is_quad {
            spans.push((parts[0].0, parts[3].1));
        }
    }
    replace_spans(text, &spans, "[REDACTED_IP]")
}

/// IPv6 (`fe80::1`, `2001:db8::8a2e:370:7334`, full 8-group form). The run
/// must not touch a word character on either side, so C++ scopes like
/// `FecFrameDecoder::addShard` are kept, and must be compressed (`::`) or
/// have eight groups, so clock times `12:34:56` are kept.
fn mask_ipv6(text: &str) -> String {
    let b = text.as_bytes();
    let is_v6 = |c: u8| c.is_ascii_hexdigit() || c == b':';
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !is_v6(b[i]) {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < b.len() && is_v6(b[i]) {
            i += 1;
        }
        let run_end = i;
        if (run_start > 0 && is_word_byte(b[run_start - 1]))
            || (run_end < b.len() && is_word_byte(b[run_end]))
        {
            continue;
        }
        // A single leading/trailing ':' is punctuation ("addr: fe80::1:").
        let mut start = run_start;
        let mut end = run_end;
        if end - start >= 2 && b[end - 1] == b':' && b[end - 2] != b':' {
            end -= 1;
        }
        if end - start >= 2 && b[start] == b':' && b[start + 1] != b':' {
            start += 1;
        }
        if looks_like_ipv6(&text[start..end]) {
            spans.push((start, end));
        }
    }
    replace_spans(text, &spans, "[REDACTED_IP]")
}

fn looks_like_ipv6(s: &str) -> bool {
    if s.contains(":::") || s.matches("::").count() > 1 {
        return false;
    }
    let groups: Vec<&str> = s.split(':').collect();
    if groups.len() > 9 || groups.iter().any(|g| g.len() > 4) {
        return false;
    }
    let non_empty = groups.iter().filter(|g| !g.is_empty()).count();
    if s.contains("::") {
        non_empty >= 2
    } else {
        groups.len() == 8 && non_empty == 8
    }
}

/// Collect system info string.
pub(crate) fn system_info() -> String {
    let mut info = String::new();
    info.push_str(&format!("OS: {}\n", std::env::consts::OS));
    info.push_str(&format!("Arch: {}\n", std::env::consts::ARCH));

    // GPU info via DXGI (Windows)
    #[cfg(windows)]
    {
        if let Ok(output) = std::process::Command::new("wmic")
            .args(["path", "win32_VideoController", "get", "Name"])
            .output()
        {
            let gpu = String::from_utf8_lossy(&output.stdout);
            info.push_str(&format!("GPU: {}\n", gpu.lines().nth(1).unwrap_or("Unknown").trim()));
        }
    }

    info
}

/// Export logs to a zip file. Returns the output path on success.
pub fn export_logs(adb_path: Option<&str>, device_serial: Option<&str>) -> Result<PathBuf, String> {
    let downloads = dirs_next::download_dir()
        .unwrap_or_else(|| PathBuf::from("."));
    let timestamp = chrono_lite_timestamp();
    let zip_path = downloads.join(format!("focus-vision-logs-{timestamp}.zip"));

    let file = std::fs::File::create(&zip_path)
        .map_err(|e| format!("Failed to create zip: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 1. System info
    zip.start_file("system-info.txt", options).map_err(|e| e.to_string())?;
    zip.write_all(system_info().as_bytes()).map_err(|e| e.to_string())?;

    // 2. PC-side engine logs
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let log_dir = PathBuf::from(appdata).join("FocusVisionPCVR");
        if log_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&log_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "json" || e == "log") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let fname = match path.file_name() {
                                Some(f) => f.to_string_lossy(),
                                None => continue,
                            };
                            let name = format!("pc/{}", fname);
                            let _ = zip.start_file(&name, options);
                            let _ = zip.write_all(sanitize_pii(&content).as_bytes());
                        }
                    }
                }
            }
        }
    }

    // 3. HMD logcat (if ADB available)
    if let (Some(adb), Some(serial)) = (adb_path, device_serial) {
        match adb::dump_logcat(adb, serial) {
            Ok(logcat) => {
                let _ = zip.start_file("hmd/logcat.txt", options);
                let _ = zip.write_all(sanitize_pii(&logcat).as_bytes());
            }
            Err(e) => {
                let _ = zip.start_file("hmd/logcat-error.txt", options);
                let _ = zip.write_all(format!("Failed to capture logcat: {e}").as_bytes());
            }
        }
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(zip_path)
}

pub(crate) fn chrono_lite_timestamp() -> String {
    // Simple timestamp without chrono dependency
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_address_is_redacted() {
        let result = sanitize_pii("192.168.1.1");
        assert_eq!(result, "[REDACTED_IP]");
    }

    #[test]
    fn non_ip_numbers_preserved() {
        let result = sanitize_pii("12345");
        assert_eq!(result, "12345");
    }

    #[test]
    fn system_info_returns_non_empty_string() {
        let info = system_info();
        assert!(!info.is_empty());
        assert!(info.contains("OS:"));
        assert!(info.contains("Arch:"));
    }

    #[test]
    fn chrono_lite_timestamp_returns_non_zero() {
        let ts = chrono_lite_timestamp();
        let secs: u64 = ts.parse().expect("timestamp should be a number");
        assert!(secs > 0);
    }

    #[test]
    fn multiple_ips_in_one_string_all_masked() {
        let result = sanitize_pii("src 10.0.0.1 dst 172.16.0.1 done");
        assert_eq!(result, "src [REDACTED_IP] dst [REDACTED_IP] done");
    }

    #[test]
    fn ip_with_port_ip_part_is_masked() {
        let result = sanitize_pii("192.168.1.1:9944");
        assert!(result.contains("[REDACTED_IP]"));
        // The port part should remain after the redacted IP
        assert!(result.contains(":9944"));
    }

    #[test]
    fn empty_string_sanitization_returns_empty() {
        let result = sanitize_pii("");
        assert_eq!(result, "");
    }

    // --- UTF-8 safety ---

    #[test]
    fn non_ascii_text_is_preserved() {
        // Regression: bytes were pushed one by one `as char`, turning every
        // multi-byte UTF-8 sequence into mojibake.
        let input = "接続先 192.168.1.20 に接続しました — ストリーミング開始 ✓";
        assert_eq!(
            sanitize_pii(input),
            "接続先 [REDACTED_IP] に接続しました — ストリーミング開始 ✓"
        );
        let plain = "エンジン停止: 設定を確認してください (été, naïve)";
        assert_eq!(sanitize_pii(plain), plain);
    }

    // --- IPv4 ---

    #[test]
    fn ipv4_with_sentence_period_is_masked() {
        // Regression: the trailing '.' made it 4 dots and the IP leaked.
        assert_eq!(
            sanitize_pii("Connected to 192.168.1.5."),
            "Connected to [REDACTED_IP]."
        );
    }

    #[test]
    fn ipv4_in_punctuation_and_url_is_masked() {
        assert_eq!(sanitize_pii("(10.0.0.7)"), "([REDACTED_IP])");
        assert_eq!(
            sanitize_pii("peer=http://172.16.5.4:9944/x"),
            "peer=http://[REDACTED_IP]:9944/x"
        );
    }

    #[test]
    fn version_strings_are_not_ips() {
        for s in [
            "Windows 10.0.19045.1234",
            "Focus Vision PCVR v3.0.0",
            "lib 1.2.3.4.5",
            "999.1.1.1",
            "rate 1.5",
        ] {
            assert_eq!(sanitize_pii(s), s, "{s}");
        }
    }

    // --- IPv6 ---

    #[test]
    fn ipv6_addresses_are_masked() {
        assert_eq!(sanitize_pii("addr fe80::1ff:fe23:4567:890a%wlan0"), "addr [REDACTED_IP]%wlan0");
        assert_eq!(sanitize_pii("[2001:db8::1]:9944"), "[[REDACTED_IP]]:9944");
        assert_eq!(
            sanitize_pii("full 2001:0db8:85a3:0000:0000:8a2e:0370:7334 end"),
            "full [REDACTED_IP] end"
        );
        assert_eq!(sanitize_pii("listening on fe80::1:"), "listening on [REDACTED_IP]:");
    }

    #[test]
    fn ipv6_masker_keeps_code_scopes_and_clock_times() {
        for s in [
            "FecFrameDecoder::addShard dropped shard",
            "std::vector<uint8_t> and OpenXRApp::mainLoop",
            "09-25 12:34:56.789  1234  5678 I FocusVision: started",
            "elapsed 01:02:03",
            "LOG :: separator",
        ] {
            assert_eq!(sanitize_pii(s), s, "{s}");
        }
    }

    // --- MAC ---

    #[test]
    fn mac_addresses_are_masked() {
        assert_eq!(
            sanitize_pii("BSSID: a4:5e:60:c2:11:9f, rssi -52"),
            "BSSID: [REDACTED_MAC], rssi -52"
        );
        assert_eq!(sanitize_pii("mac=A4-5E-60-C2-11-9F"), "mac=[REDACTED_MAC]");
    }

    #[test]
    fn mac_masker_ignores_mixed_separators_and_guids() {
        for s in [
            "a4:5e-60:c2:11:9f",
            "id 550e8400-e29b-41d4-a716-446655440000",
        ] {
            assert_eq!(sanitize_pii(s), s, "{s}");
        }
    }

    // --- SSID ---

    #[test]
    fn ssid_values_are_masked() {
        assert_eq!(
            sanitize_pii(r#"SSID: "HomeNet-5G", BSSID: a4:5e:60:c2:11:9f"#),
            r#"SSID: "[REDACTED_SSID]", BSSID: [REDACTED_MAC]"#
        );
        assert_eq!(sanitize_pii("ssid=CafeWifi freq=5180"), "ssid=[REDACTED_SSID] freq=5180");
        assert_eq!(sanitize_pii(r#"{"SSID":"Office"}"#), r#"{"SSID":"[REDACTED_SSID]"}"#);
        assert_eq!(sanitize_pii(r#"SSID: "我が家のWi-Fi""#), r#"SSID: "[REDACTED_SSID]""#);
    }

    #[test]
    fn ssid_masker_keeps_unrelated_words() {
        for s in ["SSID: <unknown ssid>", "scanned 3 SSIDs", "mSsid=x"] {
            assert_eq!(sanitize_pii(s), s, "{s}");
        }
    }

    // --- e-mail ---

    #[test]
    fn email_addresses_are_masked() {
        assert_eq!(
            sanitize_pii("account: taro.yamada+vr@example.co.jp."),
            "account: [REDACTED_EMAIL]."
        );
    }

    #[test]
    fn email_masker_needs_a_real_domain() {
        for s in ["user@localhost", "tag @ mention", "a@b.c"] {
            assert_eq!(sanitize_pii(s), s, "{s}");
        }
    }

    // --- user profile paths ---

    #[test]
    fn user_profile_names_are_masked() {
        assert_eq!(
            sanitize_pii(r"C:\Users\taro\AppData\Roaming\FocusVisionPCVR"),
            r"C:\Users\[REDACTED_USER]\AppData\Roaming\FocusVisionPCVR"
        );
        assert_eq!(
            sanitize_pii(r"C:\Users\Taro Yamada\Videos"),
            r"C:\Users\[REDACTED_USER]\Videos"
        );
        assert_eq!(
            sanitize_pii(r#""dir":"C:\\Users\\taro\\rec""#),
            r#""dir":"C:\\Users\\[REDACTED_USER]\\rec""#
        );
        assert_eq!(sanitize_pii("/home/taro/.config"), "/home/[REDACTED_USER]/.config");
        assert_eq!(sanitize_pii("/Users/taro done"), "/Users/[REDACTED_USER] done");
        assert_eq!(sanitize_pii(r"c:\users\山田\x"), r"c:\users\[REDACTED_USER]\x");
    }

    // --- pairing PIN ---

    #[test]
    fn pairing_pin_is_masked() {
        assert_eq!(sanitize_pii("Pairing PIN: 048217"), "Pairing PIN: [REDACTED_PIN]");
        assert_eq!(
            sanitize_pii(r#"{"status":"waiting","pin":"048217"}"#),
            r#"{"status":"waiting","pin":"[REDACTED_PIN]"}"#
        );
        assert_eq!(sanitize_pii("pin=0421"), "pin=[REDACTED_PIN]");
    }

    #[test]
    fn pin_masker_keeps_protocol_words_and_sentinel() {
        for s in [
            "step PIN_RESPONSE read failed",
            "PIN accepted",
            r#""pin":"------""#,
            "spin: 123456",
        ] {
            assert_eq!(sanitize_pii(s), s, "{s}");
        }
    }
}
