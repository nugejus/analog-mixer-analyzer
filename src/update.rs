//! Update check against a `latest.txt` on Google Drive, via the system `curl`.
use std::path::Path;
use std::process::Command;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LATEST_URL: &str = "https://drive.google.com/uc?export=download&id=1794QcL_4W-HXkp3ayiL_lf5Q4qpENQ-y";
pub const FOLDER_URL: &str = "https://drive.google.com/drive/folders/16sIClrgZI2zEmbxvKF1BxcOzVhQ9OcIM";

#[derive(Clone, Debug, PartialEq)]
pub struct Latest {
    pub version: String,
    pub url: String,
}

/// "1.2.3" -> [1, 2, 3]; None if any part is not a number.
fn nums(v: &str) -> Option<Vec<u32>> {
    v.split('.').map(|p| p.parse().ok()).collect()
}

/// First two non-empty lines: a dotted version, then an https URL. Anything else is `None`.
pub fn parse_latest(text: &str) -> Option<Latest> {
    let mut lines = text.trim_start_matches('\u{feff}').lines().map(str::trim).filter(|l| !l.is_empty());
    let (version, url) = (lines.next()?, lines.next()?);
    nums(version)?;
    url.starts_with("https://").then(|| Latest { version: version.into(), url: url.into() })
}

/// True only when both versions are numeric and `latest` is greater.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((nums(latest), nums(current)), (Some(a), Some(b)) if a > b)
}

fn curl() -> Command {
    let mut c = Command::new("curl");
    c.arg("-fsSL");
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// Any failure (offline, no curl, bad format) is `None`.
pub fn fetch_latest() -> Option<Latest> {
    let out = curl().args(["--max-time", "10", LATEST_URL]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_latest(&String::from_utf8_lossy(&out.stdout))
}

pub fn download(url: &str, dest: &Path) -> std::io::Result<()> {
    let out = curl().args(["--max-time", "300", "-o"]).arg(dest).arg(url).output()?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    Err(std::io::Error::other(if err.is_empty() {
        format!("curl 종료 코드 {}", out.status.code().map_or("?".into(), |c| c.to_string()))
    } else {
        err.to_string()
    }))
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
    Command::new(opener).arg(url).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numbers() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("abc", "0.2.0"));
        assert!(!is_newer("0.3.0", "x"));
    }

    #[test]
    fn parse_reads_two_lines() {
        let l = parse_latest("0.3.0\nhttps://drive.google.com/uc?export=download&id=A\n").unwrap();
        assert_eq!(l, Latest { version: "0.3.0".into(), url: "https://drive.google.com/uc?export=download&id=A".into() });
    }

    #[test]
    fn parse_ignores_bom_crlf_spaces() {
        let l = parse_latest("\u{feff} 0.3.0 \r\n\r\n https://x.example/a \r\n").unwrap();
        assert_eq!((l.version.as_str(), l.url.as_str()), ("0.3.0", "https://x.example/a"));
    }

    #[test]
    fn parse_rejects_bad_text() {
        assert_eq!(parse_latest(""), None);
        assert_eq!(parse_latest("0.3.0\n"), None);
        assert_eq!(parse_latest("<!DOCTYPE html>\n<html><body>Sorry</body></html>"), None);
        assert_eq!(parse_latest("v0.3\nhttps://x.example/a"), None);
        assert_eq!(parse_latest("0.3.0\nhttp://x.example/a"), None);
    }

    /// Network: run with `cargo test --lib update -- --ignored`.
    #[test]
    #[ignore]
    fn drive_latest_parses() {
        assert!(fetch_latest().is_some());
    }
}
