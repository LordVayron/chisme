//! The Support page's sponsor list: `sponsors.json` and the logos in the
//! `sponsors/` folder of Chisme's GitHub repo. Loaded only when the page
//! opens, so editing that file updates every copy of the app.
//!
//! Tests point it at a local server with CHISME_SPONSORS_URL (the JSON);
//! logos are read next to it.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::i18n::l;

const LIST: &str = "https://raw.githubusercontent.com/LordVayron/chisme/main/sponsors.json";
/// Where a sponsor can sign up when the list can't be read.
pub const SIGN_UP: &str = "https://github.com/sponsors/LordVayron";
/// The tiers and how to become a sponsor.
pub const TIERS: &str = "https://github.com/LordVayron/chisme/blob/main/SPONSORS.md";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// $500 a month: a large logo at the top.
    Main,
    /// $100: a logo.
    Partner,
    /// $25: name and link.
    Backer,
    /// $5: name.
    #[default]
    Supporter,
}

/// A logo decoded to RGBA.
#[derive(Clone, Debug, PartialEq)]
pub struct Logo {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Sponsor {
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub tier: Tier,
    /// A PNG or JPEG in the repo, e.g. "sponsors/acme.png".
    #[serde(default)]
    pub logo: String,
    #[serde(skip)]
    pub image: Option<Logo>,
}

impl Sponsor {
    /// The link, when it's a web page.
    pub fn link(&self) -> Option<&str> {
        self.url.starts_with("https://").then_some(self.url.as_str())
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct List {
    /// The "Become a sponsor" page.
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub sponsors: Vec<Sponsor>,
}

impl List {
    pub fn sign_up(&self) -> &str {
        if self.link.starts_with("https://") { &self.link } else { SIGN_UP }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum State {
    #[default]
    Idle,
    Loading,
    Ready(List),
    Failed(String),
}

pub type Shared = Arc<Mutex<State>>;

fn list_url() -> String {
    std::env::var("CHISME_SPONSORS_URL").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| LIST.into())
}

/// The list from its JSON, sorted by tier (the file's order within one).
pub fn parse(json: &str) -> Result<List> {
    let mut list: List = serde_json::from_str(json).context(l("reading the sponsor list", "leyendo la lista de sponsors"))?;
    list.sponsors.retain(|s| !s.name.trim().is_empty());
    list.sponsors.sort_by_key(|s| s.tier);
    Ok(list)
}

/// A logo's address: only a file in the same folder tree as the list.
pub fn logo_url(base: &str, path: &str) -> Option<String> {
    let ok = !path.is_empty() && !path.contains("..") && !path.contains(':') && !path.starts_with('/') && !path.contains('\\');
    let dir = &base[..base.rfind('/')? + 1];
    ok.then(|| format!("{dir}{path}"))
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut resp = ureq::get(url).header("User-Agent", concat!("Chisme/", env!("CARGO_PKG_VERSION"))).call()?;
    Ok(resp.body_mut().with_config().limit(limit).read_to_vec()?)
}

/// Decode a logo, scaled to fit 480x160.
fn decode(bytes: &[u8]) -> Option<Logo> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > 480 || img.height() > 160 { img.resize(480, 160, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    Some(Logo { size: [rgba.width() as usize, rgba.height() as usize], rgba: rgba.into_raw() })
}

/// The list and its logos (a logo that fails shows the name instead).
pub fn load() -> Result<List> {
    let url = list_url();
    let json = get(&url, 256 * 1024).context(l("loading the sponsor list", "cargando la lista de sponsors"))?;
    let mut list = parse(&String::from_utf8_lossy(&json))?;
    for s in &mut list.sponsors {
        if matches!(s.tier, Tier::Main | Tier::Partner)
            && let Some(u) = logo_url(&url, &s.logo)
        {
            s.image = get(&u, 1024 * 1024).ok().and_then(|b| decode(&b));
        }
    }
    Ok(list)
}

/// Load in the background (unless loading or loaded).
pub fn spawn_load(state: &Shared) {
    {
        let mut s = state.lock().unwrap();
        if matches!(*s, State::Loading | State::Ready(_)) {
            return;
        }
        *s = State::Loading;
    }
    let state = state.clone();
    std::thread::spawn(move || {
        let s = match load() {
            Ok(list) => State::Ready(list),
            Err(e) => State::Failed(format!("{e:#}")),
        };
        *state.lock().unwrap() = s;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts() {
        let list = parse(
            r#"{"link":"https://github.com/sponsors/x","sponsors":[
                {"name":"Ana","tier":"supporter"},
                {"name":"Acme","url":"https://acme.example","tier":"partner","logo":"sponsors/acme.png"},
                {"name":" "},
                {"name":"Big","tier":"main"},
                {"name":"Bob","url":"javascript:alert(1)","tier":"backer"}]}"#,
        )
        .unwrap();
        let names: Vec<&str> = list.sponsors.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Big", "Acme", "Bob", "Ana"]);
        assert_eq!(list.sponsors[1].link(), Some("https://acme.example"));
        assert_eq!(list.sponsors[2].link(), None);
        assert_eq!(list.sign_up(), "https://github.com/sponsors/x");
        assert_eq!(parse(r#"{"sponsors":[]}"#).unwrap().sign_up(), SIGN_UP);
    }

    #[test]
    fn logos_stay_in_the_repo() {
        let base = "https://raw.githubusercontent.com/LordVayron/chisme/main/sponsors.json";
        assert_eq!(logo_url(base, "sponsors/a.png").as_deref(), Some("https://raw.githubusercontent.com/LordVayron/chisme/main/sponsors/a.png"));
        for bad in ["", "../x.png", "https://evil.example/a.png", "/a.png", "sponsors\\a.png"] {
            assert_eq!(logo_url(base, bad), None, "{bad}");
        }
    }
}
