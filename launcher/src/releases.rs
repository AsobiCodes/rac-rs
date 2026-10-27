//! GitHub Releases as a source of files: the latest release of a repo (tag, notes, assets with size + sha256 digest) and a checked download.
//! Two sources behind one type, so the same code runs in tests without a network:
//! - `ReleaseSource::GitHub`: `https://api.github.com/repos/<owner>/<repo>/releases/latest`.
//! - `ReleaseSource::Folder`: a folder holding `latest.json` (same shape as the API answer) and the asset files next to it
//!   (`browser_download_url` may be just a file name). Used by the tests and by the `RAC_*_RELEASES` / `--update-base` overrides.
//! Every failure is a `ReleaseError` with a plain-English text; nothing here panics, and a repo that does not exist (404) is the same as "no release".
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const API_BASE: &str = "https://api.github.com/repos";
pub const GAME_REPO: (&str, &str) = ("AsobiCodes", "rac-rs");
pub const LAUNCHER_REPO: (&str, &str) = ("AsobiCodes", "rac-rs-launcher");

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    pub browser_download_url: String,
    /// "sha256:<hex>" (GitHub fills it for every asset uploaded since 2025).
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    /// "2026-10-21T18:00:00Z"
    #[serde(default)]
    pub published_at: Option<String>,
}

impl Release {
    /// "2026-10-21" (empty when unknown).
    pub fn date(&self) -> String {
        self.published_at.clone().unwrap_or_default().chars().take(10).collect()
    }
    /// "v0.4.0" -> "0.4.0".
    pub fn version(&self) -> String {
        self.tag_name.trim().trim_start_matches(['v', 'V']).to_string()
    }
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
    pub fn notes(&self) -> String {
        self.body.clone().unwrap_or_default().replace('\r', "")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleaseError {
    /// No network, DNS or TLS failure, timeout.
    Offline,
    /// 404: the repo or the release does not exist (yet).
    NotFound,
    RateLimited,
    Http(u16),
    /// The answer was not what we expect.
    Bad(String),
    NoDigest,
    DigestMismatch,
    DiskFull,
    Io(String),
    Cancelled,
}

impl ReleaseError {
    pub fn text(&self) -> String {
        match self {
            ReleaseError::Offline => "Could not reach GitHub. Check your internet connection and try again.".into(),
            ReleaseError::NotFound => "There is no release to download there yet.".into(),
            ReleaseError::RateLimited => "GitHub says too many requests came from your network. Wait a few minutes and try again.".into(),
            ReleaseError::Http(c) => format!("GitHub answered with error {c}. Try again later."),
            ReleaseError::Bad(m) => format!("The release information could not be read ({m})."),
            ReleaseError::NoDigest => "The download has no checksum, so it was not used. Nothing was changed.".into(),
            ReleaseError::DigestMismatch => "The downloaded file did not match its checksum (damaged or changed), so it was thrown away. Try again.".into(),
            ReleaseError::DiskFull => "The disk is full. Free some space (or pick another folder) and try again.".into(),
            ReleaseError::Io(m) => format!("A file could not be written: {m}"),
            ReleaseError::Cancelled => "Cancelled.".into(),
        }
    }
}

impl std::fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

/// Maps a write error to the plain case "disk full" where the OS says so (Windows 112 / 39, Linux 28).
pub fn io_error(e: &std::io::Error) -> ReleaseError {
    if e.kind() == std::io::ErrorKind::StorageFull || matches!(e.raw_os_error(), Some(112) | Some(39) | Some(28)) {
        ReleaseError::DiskFull
    } else {
        ReleaseError::Io(e.to_string())
    }
}

#[derive(Clone, Debug)]
pub enum ReleaseSource {
    GitHub { owner: String, repo: String },
    Folder(PathBuf),
}

impl ReleaseSource {
    pub fn github((owner, repo): (&str, &str)) -> ReleaseSource {
        ReleaseSource::GitHub { owner: owner.into(), repo: repo.into() }
    }
    /// `over` = a test folder (from a flag or an environment variable); empty counts as none.
    pub fn from_override(over: Option<&str>, repo: (&str, &str)) -> ReleaseSource {
        match over.map(str::trim).filter(|s| !s.is_empty()) {
            Some(dir) => ReleaseSource::Folder(PathBuf::from(dir)),
            None => ReleaseSource::github(repo),
        }
    }

    pub fn latest(&self) -> Result<Release, ReleaseError> {
        let text = match self {
            ReleaseSource::Folder(dir) => std::fs::read_to_string(dir.join("latest.json")).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ReleaseError::NotFound
                } else {
                    ReleaseError::Io(e.to_string())
                }
            })?,
            ReleaseSource::GitHub { owner, repo } => {
                let url = format!("{API_BASE}/{owner}/{repo}/releases/latest");
                let mut resp = http_get(&url, Some("application/vnd.github+json"))?;
                resp.body_mut().read_to_string().map_err(|_| ReleaseError::Offline)?
            }
        };
        parse_release(&text)
    }

    /// Every release of the repo, newest first as the server gives them (GitHub: `/releases?per_page=100`, up to 3 pages; folder: `releases.json`,
    /// an array in the same shape, else just `latest.json`).
    pub fn list(&self) -> Result<Vec<Release>, ReleaseError> {
        match self {
            ReleaseSource::Folder(dir) => match std::fs::read_to_string(dir.join("releases.json")) {
                Ok(t) => parse_release_list(&t),
                Err(_) => Ok(vec![self.latest()?]),
            },
            ReleaseSource::GitHub { owner, repo } => {
                let mut all = Vec::new();
                for page in 1..=3 {
                    let url = format!("{API_BASE}/{owner}/{repo}/releases?per_page=100&page={page}");
                    let mut resp = http_get(&url, Some("application/vnd.github+json"))?;
                    let text = resp.body_mut().read_to_string().map_err(|_| ReleaseError::Offline)?;
                    let part = parse_release_list(&text)?;
                    let n = part.len();
                    all.extend(part);
                    if n < 100 {
                        break;
                    }
                }
                Ok(all)
            }
        }
    }

    /// Opens the asset for reading; the second value is the length the server announced.
    fn open(&self, a: &Asset) -> Result<(Box<dyn Read + Send>, Option<u64>), ReleaseError> {
        match self {
            ReleaseSource::Folder(dir) => {
                let p = if a.browser_download_url.starts_with("http") { dir.join(&a.name) } else { dir.join(&a.browser_download_url) };
                let f = File::open(&p).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { ReleaseError::NotFound } else { ReleaseError::Io(e.to_string()) })?;
                let len = f.metadata().ok().map(|m| m.len());
                Ok((Box::new(f), len))
            }
            ReleaseSource::GitHub { .. } => {
                let resp = http_get(&a.browser_download_url, None)?;
                let len = resp.body().content_length();
                Ok((Box::new(resp.into_body().into_reader()), len))
            }
        }
    }
}

pub fn parse_release(text: &str) -> Result<Release, ReleaseError> {
    let r: Release = serde_json::from_str(text).map_err(|e| ReleaseError::Bad(e.to_string()))?;
    if r.tag_name.trim().is_empty() {
        return Err(ReleaseError::Bad("no tag".into()));
    }
    Ok(r)
}

pub fn parse_release_list(text: &str) -> Result<Vec<Release>, ReleaseError> {
    let v: Vec<Release> = serde_json::from_str(text).map_err(|e| ReleaseError::Bad(e.to_string()))?;
    Ok(v.into_iter().filter(|r| !r.tag_name.trim().is_empty()).collect())
}

fn agent() -> ureq::Agent {
    let cfg = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_resolve(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent("rac-rs-launcher");
    #[cfg(windows)]
    let cfg = cfg.tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build());
    cfg.build().into()
}

fn http_get(url: &str, accept: Option<&str>) -> Result<ureq::http::Response<ureq::Body>, ReleaseError> {
    let mut req = agent().get(url);
    if let Some(a) = accept {
        req = req.header("Accept", a);
    }
    let resp = req.call().map_err(|_| ReleaseError::Offline)?;
    let code = resp.status().as_u16();
    match code {
        200..=299 => Ok(resp),
        404 => Err(ReleaseError::NotFound),
        429 => Err(ReleaseError::RateLimited),
        403 if resp.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()) == Some("0") => Err(ReleaseError::RateLimited),
        c => Err(ReleaseError::Http(c)),
    }
}

/// Shared between the download thread and the UI.
#[derive(Default)]
pub struct Transfer {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Transfer {
    pub fn new() -> Arc<Transfer> {
        Arc::new(Transfer::default())
    }
}

/// Speed over the whole transfer so far, in bytes per second.
pub fn speed(done: u64, since: Instant) -> f64 {
    let s = since.elapsed().as_secs_f64();
    if s < 0.25 {
        0.0
    } else {
        done as f64 / s
    }
}

pub fn digest_hex(a: &Asset) -> Option<String> {
    let d = a.digest.as_deref()?.trim();
    let h = d.strip_prefix("sha256:")?.trim().to_ascii_lowercase();
    if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(h)
    } else {
        None
    }
}

/// Downloads `asset` to `dest`: streams into `dest + ".part"` (progress in `t`, stops on `t.cancel`), checks the SHA-256 against the
/// release's digest (a missing or wrong digest refuses the file and deletes the part), then renames to `dest`.
pub fn download(src: &ReleaseSource, asset: &Asset, dest: &Path, t: &Transfer) -> Result<(), ReleaseError> {
    let want = digest_hex(asset).ok_or(ReleaseError::NoDigest)?;
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let r = download_part(src, asset, &part, &want, t);
    match r {
        Ok(()) => {
            let _ = std::fs::remove_file(dest);
            std::fs::rename(&part, dest).map_err(|e| {
                let _ = std::fs::remove_file(&part);
                io_error(&e)
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn download_part(src: &ReleaseSource, asset: &Asset, part: &Path, want: &str, t: &Transfer) -> Result<(), ReleaseError> {
    if let Some(dir) = part.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_error(&e))?;
    }
    let (mut rd, len) = src.open(asset)?;
    t.done.store(0, Ordering::Relaxed);
    t.total.store(len.filter(|l| *l > 0).unwrap_or(asset.size), Ordering::Relaxed);
    let mut out = File::create(part).map_err(|e| io_error(&e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        if t.cancel.load(Ordering::Relaxed) {
            return Err(ReleaseError::Cancelled);
        }
        let n = match rd.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(if matches!(src, ReleaseSource::GitHub { .. }) { ReleaseError::Offline } else { io_error(&e) }),
        };
        h.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| io_error(&e))?;
        done += n as u64;
        t.done.store(done, Ordering::Relaxed);
    }
    out.flush().map_err(|e| io_error(&e))?;
    drop(out);
    if asset.size > 0 && done != asset.size {
        return Err(ReleaseError::DigestMismatch);
    }
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        return Err(ReleaseError::DigestMismatch);
    }
    Ok(())
}

/// SHA-256 of a file as lowercase hex (for building test releases and `build_release`-style checks).
pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Test helper (also used by the integration tests): writes a release folder with `files` (name, bytes), the given tag and notes;
/// `digest_for` lets a test break or drop a digest.
pub fn write_test_release(dir: &Path, tag: &str, body: &str, files: &[(&str, &[u8])], digest_for: &dyn Fn(&str, &str) -> Option<String>) {
    std::fs::create_dir_all(dir).unwrap();
    let mut assets = Vec::new();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
        let hex = sha256_file(&dir.join(name)).unwrap();
        let digest = digest_for(name, &hex);
        assets.push(serde_json::json!({ "name": name, "size": bytes.len(), "browser_download_url": name, "digest": digest }));
    }
    let j = serde_json::json!({ "tag_name": tag, "body": body, "assets": assets });
    std::fs::write(dir.join("latest.json"), serde_json::to_string_pretty(&j).unwrap()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_rel_{n}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }
    fn good(_: &str, hex: &str) -> Option<String> {
        Some(format!("sha256:{hex}"))
    }

    #[test]
    fn parses_the_github_shape() {
        let j = r#"{"url":"x","tag_name":"v0.4.0","name":"n","body":"Fixed\r\nthings","assets":[
            {"name":"rac-rs-0.4.0.zip","size":12,"browser_download_url":"https://github.com/a/b/releases/download/v0.4.0/rac-rs-0.4.0.zip","digest":"sha256:abc","id":5},
            {"name":"old.zip","size":1,"browser_download_url":"u","digest":null}]}"#;
        let r = parse_release(j).unwrap();
        assert_eq!((r.version().as_str(), r.assets.len()), ("0.4.0", 2));
        assert_eq!(r.notes(), "Fixed\nthings");
        assert_eq!(r.asset("rac-rs-0.4.0.zip").unwrap().size, 12);
        assert!(r.asset("old.zip").unwrap().digest.is_none());
        assert!(matches!(parse_release("{}"), Err(ReleaseError::Bad(_))));
        assert!(matches!(parse_release("not json"), Err(ReleaseError::Bad(_))));
    }

    #[test]
    fn digest_text_is_strict() {
        let mk = |d: Option<&str>| Asset { name: "a".into(), size: 1, browser_download_url: "u".into(), digest: d.map(String::from) };
        let hex = "0".repeat(64);
        assert_eq!(digest_hex(&mk(Some(&format!("sha256:{hex}")))), Some(hex.clone()));
        assert_eq!(digest_hex(&mk(Some(&format!("SHA256:{hex}")))), None);
        assert_eq!(digest_hex(&mk(Some("sha256:abc"))), None);
        assert_eq!(digest_hex(&mk(None)), None);
    }

    #[test]
    fn folder_source_downloads_and_verifies() {
        let d = scratch("ok");
        let data: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        write_test_release(&d, "v1.2.3", "notes", &[("tool.exe", &data)], &good);
        let src = ReleaseSource::Folder(d.clone());
        let rel = src.latest().unwrap();
        assert_eq!(rel.version(), "1.2.3");
        let dest = d.join("out").join("tool.exe");
        let t = Transfer::default();
        download(&src, rel.asset("tool.exe").unwrap(), &dest, &t).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(t.done.load(Ordering::Relaxed), data.len() as u64);
        assert!(!d.join("out").join("tool.exe.part").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wrong_or_missing_digest_is_refused_and_nothing_is_left() {
        let d = scratch("bad");
        write_test_release(&d, "v1.0.0", "", &[("a.bin", b"hello world"), ("b.bin", b"hello world")], &|n, hex| {
            if n == "a.bin" {
                let _ = hex;
                Some(format!("sha256:{}", "0".repeat(64)))
            } else {
                None
            }
        });
        let src = ReleaseSource::Folder(d.clone());
        let rel = src.latest().unwrap();
        let t = Transfer::default();
        let dest_a = d.join("o").join("a.bin");
        assert_eq!(download(&src, rel.asset("a.bin").unwrap(), &dest_a, &t), Err(ReleaseError::DigestMismatch));
        assert!(!dest_a.exists() && !d.join("o").join("a.bin.part").exists());
        let dest_b = d.join("o").join("b.bin");
        assert_eq!(download(&src, rel.asset("b.bin").unwrap(), &dest_b, &t), Err(ReleaseError::NoDigest));
        assert!(!dest_b.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cancel_stops_and_removes_the_part() {
        let d = scratch("cancel");
        write_test_release(&d, "v1.0.0", "", &[("a.bin", &vec![7u8; 1_000_000])], &good);
        let src = ReleaseSource::Folder(d.clone());
        let rel = src.latest().unwrap();
        let t = Transfer::default();
        t.cancel.store(true, Ordering::Relaxed);
        let dest = d.join("o").join("a.bin");
        assert_eq!(download(&src, &rel.assets[0], &dest, &t), Err(ReleaseError::Cancelled));
        assert!(!dest.exists() && !d.join("o").join("a.bin.part").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_folder_is_not_found_and_override_picks_the_source() {
        let src = ReleaseSource::Folder(scratch("none"));
        assert_eq!(src.latest(), Err(ReleaseError::NotFound));
        assert!(matches!(ReleaseSource::from_override(Some(" "), GAME_REPO), ReleaseSource::GitHub { .. }));
        assert!(matches!(ReleaseSource::from_override(Some("D:\\x"), GAME_REPO), ReleaseSource::Folder(_)));
        assert!(ReleaseError::RateLimited.text().contains("too many requests"));
        assert_eq!(io_error(&std::io::Error::from_raw_os_error(112)), ReleaseError::DiskFull);
    }
}
