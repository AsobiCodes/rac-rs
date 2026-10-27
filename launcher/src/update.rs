//! Update check: the same sources and rules as setup.ps1 (Read-Remote, Get-ChangelogFor, Compare-Version).
//! VERSION and CHANGELOG.md come from github.com/AsobiCodes/rac-rs (main); the update itself is done by the setup.ps1 -ApplyUpdate worker
//! (see engine.rs). A test base (folder, file:// or http URL that serves VERSION and CHANGELOG.md) replaces GitHub.
use crate::games::is_version;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const REPO_RAW: &str = "https://raw.githubusercontent.com/AsobiCodes/rac-rs/main";

/// -1 / 0 / 1 like [version]::CompareTo; unreadable parts count as 0. A test suffix ("0.4.0-rc1") sorts just below the same number without it.
pub fn compare_version(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> (Vec<u64>, bool) {
        let s = s.trim().trim_start_matches('v');
        let (num, pre) = match s.split_once('-') {
            Some((n, _)) => (n, true),
            None => (s, false),
        };
        (num.split('.').map(|x| x.parse().unwrap_or(0)).collect(), pre)
    };
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    na.cmp(&nb).then_with(|| pb.cmp(&pa))
}

/// Text of VERSION / CHANGELOG.md; None on any failure, never panics.
pub fn read_remote(base: Option<&str>, name: &str, timeout: Duration) -> Option<String> {
    let base = base.unwrap_or(REPO_RAW);
    if let Some(rest) = base.strip_prefix("file://") {
        let rest = rest.trim_start_matches('/');
        return std::fs::read_to_string(Path::new(rest).join(name)).ok();
    }
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return std::fs::read_to_string(Path::new(base).join(name)).ok();
    }
    http_get(&format!("{}/{}", base.trim_end_matches('/'), name), timeout)
}

fn http_get(url: &str, timeout: Duration) -> Option<String> {
    let cfg = ureq::Agent::config_builder().timeout_global(Some(timeout)).user_agent("rac-rs-launcher");
    #[cfg(windows)]
    let cfg = cfg.tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build());
    let agent: ureq::Agent = cfg.build().into();
    let mut resp = agent.get(url).call().ok()?;
    resp.body_mut().read_to_string().ok()
}

/// CHANGELOG.md has one section per version, newest first, starting with "## v<version>". Returns the versions newer than `from` up to `to`,
/// as the old window showed them ("Version x.y.z", bullets with a dot).
pub fn changelog_for(text: &str, from: &str, to: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut take = false;
    for l in text.lines() {
        if let Some(h) = l.strip_prefix("##") {
            let h = h.trim_start();
            let h = h.strip_prefix('v').unwrap_or(h);
            let ver: String = h.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if is_version(&ver) {
                use std::cmp::Ordering::*;
                take = compare_version(&ver, from) == Greater && compare_version(&ver, to) != Greater;
                if take {
                    if !out.is_empty() {
                        out.push(String::new());
                    }
                    out.push(format!("Version {ver}"));
                }
                continue;
            }
        }
        if take && !l.trim().is_empty() {
            let t = l.trim_start();
            let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).map(|r| format!("\u{2022} {}", r.trim_start())).unwrap_or_else(|| l.to_string());
            out.push(t);
        }
    }
    out.join("\n")
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateOffer {
    pub from: String,
    pub to: String,
    pub changes: String,
    /// An earlier update is unfinished: only the rebuild is left (no download).
    pub finish: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Checking,
    UpToDate(String),
    Available(UpdateOffer),
    /// Could not reach the server (offline): no banner.
    Offline,
}

/// What the Play screen shows: the offer for (local, built) versions against the remote text. `remote_version` None = offline.
pub fn evaluate(local: &str, built: &str, remote_version: Option<&str>, changelog: Option<&str>) -> UpdateState {
    let mut state = UpdateState::Offline;
    if let Some(rv) = remote_version.map(|s| s.trim()).filter(|v| is_version(v)) {
        use std::cmp::Ordering::*;
        state = UpdateState::UpToDate(local.to_string());
        if compare_version(rv, local) == Greater {
            let from = if !built.is_empty() && compare_version(built, local) == Less { built } else { local };
            let changes = changelog.map(|c| changelog_for(c, from, rv)).filter(|t| !t.is_empty()).unwrap_or_else(|| "(The list of changes could not be loaded.)".to_string());
            return UpdateState::Available(UpdateOffer { from: from.to_string(), to: rv.to_string(), changes, finish: false });
        }
    }
    // the files are newer than the build (an unfinished update): offer the rebuild, with the local change list
    if !built.is_empty() && built != local && compare_version(built, local) == std::cmp::Ordering::Less {
        let changes = changelog.map(|c| changelog_for(c, built, local)).filter(|t| !t.is_empty()).unwrap_or_default();
        return UpdateState::Available(UpdateOffer { from: built.to_string(), to: local.to_string(), changes, finish: true });
    }
    state
}

/// Runs the check on a thread; the result lands in `slot`.
pub fn check_async(local: String, built: String, base: Option<String>, root: std::path::PathBuf, slot: Arc<Mutex<UpdateState>>) {
    std::thread::spawn(move || {
        let t = Duration::from_millis(4000);
        let rv = read_remote(base.as_deref(), "VERSION", t);
        let local_log = std::fs::read_to_string(root.join("CHANGELOG.md")).ok();
        let log = if rv.as_deref().map(|v| is_version(v.trim()) && compare_version(v.trim(), &local) == std::cmp::Ordering::Greater).unwrap_or(false) {
            read_remote(base.as_deref(), "CHANGELOG.md", t)
        } else {
            local_log
        };
        let st = evaluate(&local, &built, rv.as_deref(), log.as_deref());
        if let Ok(mut g) = slot.lock() {
            *g = st;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    const LOG: &str = "# Changelog\n\n## v0.4.0\n- Fixed the thing\n- Second fix\n\n## v0.3.1\n* AMD start-up fix\n\n## v0.3.0\n- Big release\n";

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_version("0.3.10", "0.3.9"), Ordering::Greater);
        assert_eq!(compare_version("0.3.1", "0.3.1"), Ordering::Equal);
        assert_eq!(compare_version("0.2.9", "0.3.0"), Ordering::Less);
    }

    #[test]
    fn change_list_is_the_versions_between() {
        let t = changelog_for(LOG, "0.3.0", "0.4.0");
        assert_eq!(t, "Version 0.4.0\n\u{2022} Fixed the thing\n\u{2022} Second fix\n\nVersion 0.3.1\n\u{2022} AMD start-up fix");
        assert_eq!(changelog_for(LOG, "0.3.1", "0.3.1"), "");
        assert_eq!(changelog_for(LOG, "0.3.1", "0.4.0"), "Version 0.4.0\n\u{2022} Fixed the thing\n\u{2022} Second fix");
    }

    #[test]
    fn offer_when_the_remote_is_newer() {
        match evaluate("0.3.1", "0.3.1", Some("0.4.0\n"), Some(LOG)) {
            UpdateState::Available(o) => {
                assert_eq!((o.from.as_str(), o.to.as_str(), o.finish), ("0.3.1", "0.4.0", false));
                assert!(o.changes.starts_with("Version 0.4.0"));
            }
            s => panic!("{s:?}"),
        }
        assert_eq!(evaluate("0.4.0", "0.4.0", Some("0.4.0"), Some(LOG)), UpdateState::UpToDate("0.4.0".into()));
        assert_eq!(evaluate("0.4.0", "0.4.0", None, None), UpdateState::Offline);
        assert_eq!(evaluate("0.4.0", "0.4.0", Some("garbage"), None), UpdateState::Offline);
    }

    #[test]
    fn unfinished_update_uses_the_built_version_as_start() {
        // files are 0.4.0 but the game exe is still 0.3.0, the server says 0.3.6: from = built
        match evaluate("0.4.0", "0.3.0", Some("0.3.6"), Some("## v0.3.6\n- x\n## v0.4.0\n- y\n## v0.3.1\n- z\n")) {
            UpdateState::Available(o) => assert_eq!(o.from, "0.3.0"),
            s => panic!("{s:?}"),
        }
        // offline: the local rebuild is still offered
        match evaluate("0.4.0", "0.3.0", None, Some(LOG)) {
            UpdateState::Available(o) => assert!(o.finish && o.to == "0.4.0"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn test_base_folder_and_file_url() {
        let d = std::env::temp_dir().join(format!("racl_upd_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("VERSION"), "9.9.9\n").unwrap();
        let t = Duration::from_millis(100);
        assert_eq!(read_remote(Some(d.to_str().unwrap()), "VERSION", t).as_deref(), Some("9.9.9\n"));
        let url = format!("file:///{}", d.to_str().unwrap().replace('\\', "/"));
        assert_eq!(read_remote(Some(&url), "VERSION", t).as_deref(), Some("9.9.9\n"));
        assert_eq!(read_remote(Some(d.to_str().unwrap()), "NOPE", t), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
