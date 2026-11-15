//! Output helpers shared by the CLI commands.

use serde::Serialize;

/// Print a value either as pretty JSON or via a human readable closure.
pub fn emit<T: Serialize>(json: bool, value: &T, human: impl FnOnce(&T)) {
    if json {
        match serde_json::to_string_pretty(value) {
            Ok(text) => println!("{text}"),
            Err(err) => eprintln!("gguf: failed to serialise output: {err}"),
        }
    } else {
        human(value);
    }
}

/// Format a byte count with binary units.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_formatting_uses_binary_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(2_147_483_648), "2.0 GiB");
        assert_eq!(human_bytes(10_737_418_240), "10.0 GiB");
    }
}
