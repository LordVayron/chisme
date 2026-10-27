//! Update notice: asks GitHub for the latest release, downloads its
//! installer and runs it silently. The installer reopens Chisme
//! (`/relaunch=1`, see installer\chisme.iss).
//!
//! Tests point it at a local server with CHISME_UPDATE_URL (the JSON the
//! GitHub API returns); then the installer may come from any address.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::i18n::l;
use crate::t;

const LATEST: &str = "https://api.github.com/repos/LordVayron/chisme/releases/latest";
const DOWNLOADS: &str = "https://github.com/LordVayron/chisme/releases/download/";

/// A newer release.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "0.7.1" (no "v").
    pub version: String,
    /// Release notes (Markdown).
    pub notes: String,
    /// The release page.
    pub page: String,
    pub installer: String,
    pub size: u64,
    /// SHA-256 of the installer, when GitHub gives it.
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum State {
    #[default]
    Idle,
    Checking,
    /// The last check found nothing newer.
    UpToDate,
    Available(Release),
    /// Downloading (0..1).
    Downloading(Release, f32),
    /// Downloaded and checked, waiting to run.
    Ready(Release, PathBuf),
    Failed(String),
}

pub type Shared = Arc<Mutex<State>>;

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

fn test_url() -> Option<String> {
    std::env::var("CHISME_UPDATE_URL").ok().filter(|u| !u.is_empty())
}

/// "v0.7.1" -> (0, 7, 1).
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split(['-', '+']).next()?;
    let mut n = v.split('.').map(|p| p.parse::<u64>().ok());
    Some((n.next()??, n.next().unwrap_or(Some(0))?, n.next().unwrap_or(Some(0))?))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// The release in GitHub's JSON, if it's newer than `current` and has an
/// installer from the expected place.
fn pick(json: &str, current: &str, any_host: bool) -> Result<Option<Release>> {
    let r: GhRelease = serde_json::from_str(json).context(l("reading the release", "leyendo la versión"))?;
    if !is_newer(&r.tag_name, current) {
        return Ok(None);
    }
    let Some(a) = r.assets.iter().find(|a| a.name.starts_with("Chisme-Setup-") && a.name.ends_with(".exe")) else {
        bail!(t!("the release {} has no installer", "la versión {} no tiene instalador", r.tag_name));
    };
    if !any_host && !a.browser_download_url.starts_with(DOWNLOADS) {
        bail!(l("the installer isn't on Chisme's GitHub", "el instalador no está en el GitHub de Chisme"));
    }
    let sha256 = a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).map(|h| h.to_ascii_lowercase());
    Ok(Some(Release {
        version: r.tag_name.trim_start_matches(['v', 'V']).to_string(),
        notes: r.body,
        page: r.html_url,
        installer: a.browser_download_url.clone(),
        size: a.size,
        sha256,
    }))
}

/// Ask for the latest release. None: Chisme is up to date.
pub fn check() -> Result<Option<Release>> {
    let test = test_url();
    let url = test.as_deref().unwrap_or(LATEST);
    let mut resp = ureq::get(url)
        .header("User-Agent", concat!("Chisme/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .context(l("asking GitHub for the latest version", "preguntando a GitHub por la última versión"))?;
    let json = resp.body_mut().read_to_string()?;
    pick(&json, env!("CARGO_PKG_VERSION"), test.is_some())
}

/// Download the installer to a temp folder and check its size (and
/// SHA-256 when known). `progress` gets 0..1.
pub fn download(rel: &Release, progress: &dyn Fn(f32)) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("Chisme-update");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("Chisme-Setup-{}.exe", rel.version));
    let part = path.with_extension("part");
    let resp = ureq::get(&rel.installer)
        .header("User-Agent", concat!("Chisme/", env!("CARGO_PKG_VERSION")))
        .call()
        .context(l("downloading the update", "descargando la actualización"))?;
    let mut reader = resp.into_body().into_reader();
    let mut file = std::io::BufWriter::new(std::fs::File::create(&part)?);
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    let mut hash = <sha2::Sha256 as sha2::Digest>::new();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hash, &buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        if done > rel.size {
            break;
        }
        progress(done as f32 / rel.size.max(1) as f32);
    }
    file.flush()?;
    drop(file);
    if done != rel.size {
        let _ = std::fs::remove_file(&part);
        bail!(t!("the download was incomplete ({} of {} bytes)", "la descarga quedó incompleta ({} de {} bytes)", done, rel.size));
    }
    let got: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
    if rel.sha256.as_ref().is_some_and(|want| *want != got) {
        let _ = std::fs::remove_file(&part);
        bail!(l("the download doesn't match the published installer", "la descarga no coincide con el instalador publicado"));
    }
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&part, &path)?;
    Ok(path)
}

/// Silent install; the installer closes Chisme if it's still open and
/// reopens it at the end.
pub const INSTALL_ARGS: [&str; 5] = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/relaunch=1"];

pub fn install(installer: &std::path::Path) -> Result<()> {
    std::process::Command::new(installer).args(INSTALL_ARGS).spawn().context(l("starting the installer", "abriendo el instalador"))?;
    Ok(())
}

/// Check in the background.
pub fn spawn_check(state: &Shared) {
    *state.lock().unwrap() = State::Checking;
    let state = state.clone();
    std::thread::spawn(move || {
        let s = match check() {
            Ok(Some(r)) => State::Available(r),
            Ok(None) => State::UpToDate,
            Err(e) => State::Failed(format!("{e:#}")),
        };
        *state.lock().unwrap() = s;
    });
}

/// Download in the background.
pub fn spawn_download(state: &Shared, rel: Release) {
    *state.lock().unwrap() = State::Downloading(rel.clone(), 0.0);
    let state = state.clone();
    std::thread::spawn(move || {
        let s2 = state.clone();
        let r2 = rel.clone();
        let s = match download(&rel, &move |f| *s2.lock().unwrap() = State::Downloading(r2.clone(), f)) {
            Ok(p) => State::Ready(rel, p),
            Err(e) => State::Failed(format!("{e:#}")),
        };
        *state.lock().unwrap() = s;
    });
}

/// Release notes as plain lines: no headings marks, bullets as "•".
pub fn plain_notes(md: &str) -> String {
    md.lines()
        .map(|l| {
            let t = l.trim_start();
            let t = t.trim_start_matches('#').trim_start();
            match t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                Some(rest) => format!("•  {}", rest.replace("**", "").replace('`', "")),
                None => t.replace("**", "").replace('`', ""),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(tag: &str, url: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","body":"Added\n- Update notice","html_url":"https://github.com/LordVayron/chisme/releases/tag/{tag}",
            "assets":[{{"name":"notes.txt","size":3,"browser_download_url":"x"}},
                      {{"name":"Chisme-Setup-0.9.0.exe","size":5000000,"browser_download_url":"{url}","digest":"sha256:ABCD"}}]}}"#
        )
    }

    #[test]
    fn versions() {
        assert!(is_newer("v0.7.1", "0.7.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.7.0", "0.7.0"));
        assert!(!is_newer("v0.6.9", "0.7.0"));
        assert!(!is_newer("nightly", "0.7.0"));
        assert_eq!(parse_version("v1.2.3-beta"), Some((1, 2, 3)));
    }

    #[test]
    fn picks_the_installer() {
        let url = "https://github.com/LordVayron/chisme/releases/download/v0.9.0/Chisme-Setup-0.9.0.exe";
        let r = pick(&json("v0.9.0", url), "0.7.0", false).unwrap().unwrap();
        assert_eq!(r.version, "0.9.0");
        assert_eq!(r.installer, url);
        assert_eq!(r.size, 5000000);
        assert_eq!(r.sha256.as_deref(), Some("abcd"));
        assert_eq!(pick(&json("v0.7.0", url), "0.7.0", false).unwrap(), None);
        // Only from Chisme's own releases (unless testing).
        assert!(pick(&json("v0.9.0", "https://example.com/x.exe"), "0.7.0", false).is_err());
        assert!(pick(&json("v0.9.0", "http://127.0.0.1:9/x.exe"), "0.7.0", true).is_ok());
    }

    #[test]
    fn notes() {
        assert_eq!(plain_notes("## Added\n- **Update** notice\n* `x`"), "Added\n•  Update notice\n•  x");
    }
}
