//! Templates: a named layout (and optionally the brand kit), saved as
//! `%APPDATA%\Chisme\plantillas\<name>.chisme` (JSON). Exported files
//! carry their images inside (base64), so a template works on any PC.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::brand::Brand;
use crate::compose::{PipSpec, Scene};
use crate::layout::Layout;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    pub layout: Layout,
    /// The brand kit to apply with it (None keeps the current one).
    #[serde(default)]
    pub brand: Option<Brand>,
    /// Images carried inside an exported file: (file name, base64 data).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<(String, String)>,
    /// The group it is shown in on the Templates page ("" = ungrouped).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Its own 9:16 layout (None: made from `layout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<Layout>,
}

pub const EXT: &str = "chisme";

fn appdata() -> PathBuf {
    crate::settings::Settings::path().parent().map(Path::to_path_buf).unwrap_or_default()
}

pub fn dir() -> PathBuf {
    appdata().join("plantillas")
}

/// Where imported images go.
pub fn assets_dir() -> PathBuf {
    appdata().join("recursos")
}

/// A file name for a template name (letters, digits, spaces, dashes).
pub fn slug(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.trim().is_empty() { "Plantilla".into() } else { s.trim().to_string() }
}

fn path_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.{EXT}", slug(name)))
}

/// Saved templates, by name.
pub fn list_in(dir: &Path) -> Vec<Template> {
    let mut out: Vec<Template> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == EXT))
        .filter_map(|e| read(&e.path()).ok())
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    out
}

pub fn list() -> Vec<Template> {
    list_in(&dir())
}

/// Templates are small; a shared one with its pictures inside stays well
/// under this.
const MAX_FILE: u64 = 64 << 20;

fn read(path: &Path) -> Result<Template> {
    if std::fs::metadata(path)?.len() > MAX_FILE {
        bail!("{} is too big to be a Chisme template", path.display());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}')).with_context(|| format!("{} isn't a Chisme template", path.display()))?)
}

pub fn save_in(dir: &Path, t: &Template) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let p = path_in(dir, &t.name);
    let mut plain = t.clone();
    plain.assets.clear();
    std::fs::write(&p, serde_json::to_string_pretty(&plain)?)?;
    Ok(p)
}

pub fn save(t: &Template) -> Result<PathBuf> {
    save_in(&dir(), t)
}

pub fn delete(name: &str) -> Result<()> {
    std::fs::remove_file(path_in(&dir(), name))?;
    Ok(())
}

pub fn find(name: &str) -> Option<Template> {
    let n = name.trim().to_lowercase();
    let all = list();
    all.iter().find(|t| t.name.to_lowercase() == n).or_else(|| all.iter().find(|t| t.name.to_lowercase().contains(&n))).cloned()
}

/// Every image file the template uses (layers and logo).
fn images(t: &Template) -> Vec<String> {
    let mut v: Vec<String> = t.layout.layers.iter().filter_map(|l| l.image.clone()).chain(t.layout.background_image.clone()).collect();
    if let Some(logo) = t.brand.as_ref().and_then(|b| b.logo.clone()) {
        v.push(logo);
    }
    v.sort();
    v.dedup();
    v
}

/// Write a shareable file with the images inside.
pub fn export(t: &Template, to: &Path) -> Result<()> {
    let mut out = t.clone();
    out.assets = images(t)
        .into_iter()
        .filter_map(|p| {
            let data = std::fs::read(&p).ok()?;
            let name = Path::new(&p).file_name()?.to_string_lossy().into_owned();
            Some((name, base64::engine::general_purpose::STANDARD.encode(data)))
        })
        .collect();
    // Paths become plain file names, matched to the carried images.
    for l in &mut out.layout.layers {
        l.image = l.image.as_deref().and_then(|p| Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned());
    }
    if let Some(b) = &mut out.brand {
        b.logo = b.logo.as_deref().and_then(|p| Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned());
    }
    out.layout.background_image = out.layout.background_image.as_deref().and_then(|p| Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned());
    std::fs::write(to, serde_json::to_string(&out)?)?;
    Ok(())
}

/// Read a shared file: its images go to `assets` and the template is saved
/// in `templates` (renamed if one with its name exists).
pub fn import_into(from: &Path, templates: &Path, assets: &Path) -> Result<Template> {
    let mut t = read(from)?;
    if t.layout.layers.is_empty() {
        bail!("the template has no layers");
    }
    std::fs::create_dir_all(assets)?;
    t.name = t.name.chars().take(80).collect();
    let mut placed: Vec<(String, String)> = vec![];
    for (name, data) in std::mem::take(&mut t.assets) {
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).context("a damaged image in the template")?;
        // Only real PNG or JPEG pictures are written, under a name of ours
        // with the right extension (never the file's own name or type).
        let ext = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Png) => "png",
            Ok(image::ImageFormat::Jpeg) => "jpg",
            _ => continue,
        };
        let stem: String = slug(Path::new(&name).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().as_str()).chars().take(60).collect();
        let target = assets.join(format!("{}-{stem}.{ext}", slug(&t.name)));
        std::fs::write(&target, bytes)?;
        placed.push((name, target.to_string_lossy().into_owned()));
    }
    // Pictures it doesn't carry are dropped: a shared file must not point
    // Chisme at other files on this PC, or at a network share (opening
    // \\server\x.png would send the Windows login to that server).
    let resolve = |p: &str| placed.iter().find(|(n, _)| n == p).map(|(_, t)| t.clone());
    t.layout.layers.retain_mut(|l| match l.image.as_deref() {
        Some(p) => match resolve(p) {
            Some(local) => {
                l.image = Some(local);
                true
            }
            None => false,
        },
        None => true,
    });
    if t.layout.layers.is_empty() {
        bail!("the template has no layers");
    }
    if let Some(b) = &mut t.brand {
        b.logo = b.logo.as_deref().and_then(resolve);
    }
    t.layout.background_image = t.layout.background_image.as_deref().and_then(resolve);
    let base = t.name.clone();
    let mut n = 2;
    while path_in(templates, &t.name).exists() {
        t.name = format!("{base} ({n})");
        n += 1;
    }
    save_in(templates, &t)?;
    Ok(t)
}

pub fn import(from: &Path) -> Result<Template> {
    import_into(from, &dir(), &assets_dir())
}

/// Starter templates, made from the fixed scenes (1080p, 16:9 camera).
pub fn starters(pip: &PipSpec) -> Vec<Template> {
    let mk = |name: &str, scene: Scene| Template { name: name.into(), layout: Layout::from_scene(scene, pip, 1920.0, 1080.0, 16.0 / 9.0, 16.0 / 9.0), brand: None, assets: vec![], group: String::new(), vertical: None };
    // Named in the interface language when they are made.
    vec![mk(Scene::ScreenCam.label(), Scene::ScreenCam), mk(Scene::SideBySide.label(), Scene::SideBySide), mk(crate::i18n::l("Presenter", "Presentador"), Scene::CamScreen)]
}

/// Scenes for shows with guests: Interview (you and one guest), Panel (you
/// and up to three guests), Guest full screen, and Screen + guests.
pub fn guest_scenes() -> Vec<Template> {
    use crate::layout::{Layer, Source};
    let cam = |rect: [f64; 4]| Layer { source: Source::Camera, rect, fill: true, roundness: 0.06, mirror: true, ..Default::default() };
    let guest = |seat: u8, rect: [f64; 4]| Layer { source: Source::Guest, guest: seat, rect, fill: true, roundness: 0.06, ..Default::default() };
    let bg = [14u8, 14, 18];
    let mk = |name: &str, layers: Vec<Layer>| Template {
        name: name.into(),
        layout: Layout { layers, background: bg, background_image: None },
        brand: None,
        assets: vec![],
        group: crate::i18n::l("Guests", "Invitados").into(),
        vertical: None,
    };
    let (w, h) = (0.47, 0.45);
    vec![
        mk(crate::i18n::l("Interview", "Entrevista"), vec![cam([0.02, 0.25, 0.47, 0.5]), guest(0, [0.51, 0.25, 0.47, 0.5])]),
        mk(
            crate::i18n::l("Panel", "Panel"),
            vec![cam([0.02, 0.03, w, h]), guest(0, [0.51, 0.03, w, h]), guest(1, [0.02, 0.5, w, h]), guest(2, [0.51, 0.5, w, h])],
        ),
        mk(crate::i18n::l("Guest full screen", "Invitado en pantalla completa"), vec![guest(0, [0.0, 0.0, 1.0, 1.0]), cam([0.78, 0.74, 0.2, 0.22])]),
        mk(
            crate::i18n::l("Screen + guests", "Pantalla + invitados"),
            vec![Layer { source: Source::Screen, rect: [0.0, 0.0, 0.78, 1.0], fill: false, ..Default::default() }, cam([0.79, 0.02, 0.2, 0.3]), guest(0, [0.79, 0.35, 0.2, 0.3]), guest(1, [0.79, 0.68, 0.2, 0.3])],
        ),
    ]
}

/// Holding screens for a stream: "Be right back" and "Starting soon", big
/// centered text over a dark background (play a looping clip from the
/// Media Board behind it if you like).
pub fn stream_scenes() -> Vec<Template> {
    use crate::i18n::l;
    use crate::layout::Layer;
    let mk = |name: &str, title: &str, sub: &str| {
        let mut text = Layer::lower_third(title, sub, 4);
        text.rect = [0.12, 0.36, 0.76, 0.26];
        Template {
            name: name.into(),
            layout: Layout { layers: vec![text], background: [12, 12, 16], background_image: None },
            brand: None,
            assets: vec![],
            group: l("Stream", "Stream").into(),
            vertical: None,
        }
    };
    vec![
        mk(l("Be right back", "Ya volvemos"), l("Be right back", "Ya volvemos"), l("The stream continues in a moment", "La transmisión sigue en un momento")),
        mk(l("Starting soon", "Empezamos pronto"), l("Starting soon", "Empezamos pronto"), l("Grab a snack, we're about to begin", "Prepárate, ya casi empezamos")),
    ]
}

/// The Tutorial template's name.
pub fn tutorial_name() -> &'static str {
    "Tutorial"
}

/// The Tutorial look: the screen with room around it, rounded corners and
/// a shadow, on a dark gradient with an orange and a blue glow.
pub fn tutorial_scene() -> Template {
    use crate::compose::Shape;
    use crate::layout::{Layer, Source};
    let bg = assets_dir().join("tutorial-gradient.png");
    if !bg.exists() {
        let _ = std::fs::create_dir_all(assets_dir());
        let _ = gradient(1920, 1080).save(&bg);
    }
    let screen = Layer { source: Source::Screen, rect: [0.06, 0.07, 0.88, 0.86], fill: false, shape: Shape::Rounded, roundness: 0.05, shadow: true, ..Default::default() };
    Template {
        name: tutorial_name().into(),
        layout: Layout { layers: vec![screen], background: [16, 16, 22], background_image: bg.exists().then(|| bg.to_string_lossy().into_owned()) },
        brand: None,
        assets: vec![],
        group: String::new(),
        vertical: None,
    }
}

/// A dark background with soft orange (top left) and blue (bottom right)
/// glows.
fn gradient(w: u32, h: u32) -> image::RgbImage {
    let glow = |x: f64, y: f64, cx: f64, cy: f64, r: f64| (-((x - cx).powi(2) + (y - cy).powi(2)) / (r * r)).exp();
    image::RgbImage::from_fn(w, h, |x, y| {
        let (u, v) = (x as f64 / w as f64, y as f64 / h as f64 * h as f64 / w as f64);
        let o = glow(u, v, 0.1, 0.05, 0.55) * 0.55;
        let b = glow(u, v, 0.92, 0.55, 0.6) * 0.6;
        let base = [14.0, 14.0, 20.0];
        let px = [0, 1, 2].map(|k| {
            let c = base[k] + o * [255.0, 102.0, 0.0][k] * 0.6 + b * [36.0, 84.0, 255.0][k] * 0.6;
            c.clamp(0.0, 255.0) as u8
        });
        image::Rgb(px)
    })
}

/// Add the guest scenes, stream screens and Tutorial look if they aren't
/// there yet.
pub fn ensure_guest_scenes() {
    let have = list();
    let tutorial = (!have.iter().any(|h| h.name == tutorial_name())).then(tutorial_scene);
    for t in guest_scenes().into_iter().chain(stream_scenes()).chain(tutorial) {
        if !have.iter().any(|h| h.name == t.name) {
            let _ = save(&t);
        }
    }
}

/// Create the starters the first time (when there's no templates folder).
pub fn ensure_starters(pip: &PipSpec) {
    if !dir().exists() {
        for t in starters(pip) {
            let _ = save(&t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layer;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chisme-tpl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_list_and_slugs() {
        let d = tmp("list");
        for t in starters(&PipSpec::default()) {
            save_in(&d, &t).unwrap();
        }
        let names: Vec<String> = list_in(&d).into_iter().map(|t| t.name).collect();
        let mut want: Vec<String> = starters(&PipSpec::default()).into_iter().map(|t| t.name).collect();
        want.sort();
        assert_eq!(names.len(), 3);
        assert_eq!(names, want);
        assert_eq!(slug("Mi/plantilla: 1"), "Mi_plantilla_ 1");
        assert_eq!(slug("  "), "Plantilla");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn export_carries_images_and_import_restores_them() {
        let src = tmp("src");
        let logo = src.join("logo.png");
        image::RgbaImage::new(4, 4).save(&logo).unwrap();
        let png = std::fs::read(&logo).unwrap();
        let mut layout = Layout::default();
        layout.layers.push(Layer::image(&logo.to_string_lossy(), 1.0, 16.0 / 9.0));
        let t = Template { name: "Mi marca".into(), layout, brand: Some(Brand { logo: Some(logo.to_string_lossy().into()), ..Default::default() }), assets: vec![], group: String::new(), vertical: None };
        let file = src.join("Mi marca.chisme");
        export(&t, &file).unwrap();

        let (tpl, assets) = (tmp("tpl"), tmp("assets"));
        save_in(&tpl, &Template { name: "Mi marca".into(), ..t.clone() }).unwrap();
        let got = import_into(&file, &tpl, &assets).unwrap();
        assert_eq!(got.name, "Mi marca (2)", "an existing name gets a number");
        let img = got.layout.layers[1].image.clone().unwrap();
        assert!(img.starts_with(&*assets.to_string_lossy()));
        assert!(img.ends_with(".png"));
        assert_eq!(std::fs::read(&img).unwrap(), png);
        assert_eq!(got.brand.unwrap().logo.unwrap(), img, "logo and layer share the file");
        assert!(got.assets.is_empty());
        for d in [src, tpl, assets] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    /// A shared file can't make Chisme open other files (or a network
    /// share), and can't drop anything but pictures on this PC.
    #[test]
    fn import_rejects_outside_paths_and_non_images() {
        let src = tmp("evil");
        let mut layout = Layout::default(); // one screen layer
        layout.layers.push(Layer::image(r"\\attacker\share\x.png", 1.0, 1.0));
        layout.layers.push(Layer::image(r"C:\Windows\win.ini", 1.0, 1.0));
        layout.layers.push(Layer::image("run.png", 1.0, 1.0));
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let t = Template {
            name: "x".into(),
            layout,
            brand: Some(Brand { logo: Some(r"\\attacker\share\logo.png".into()), ..Default::default() }),
            assets: vec![("run.png".into(), b64(b"MZ not a picture")), (r"..\..\evil.exe".into(), b64(b"MZ"))],
            group: String::new(),
            vertical: None,
        };
        let file = src.join("x.chisme");
        std::fs::write(&file, serde_json::to_string(&t).unwrap()).unwrap();
        let (tpl, assets) = (tmp("evil-tpl"), tmp("evil-assets"));
        let got = import_into(&file, &tpl, &assets).unwrap();
        assert_eq!(got.layout.layers.len(), 1, "only the screen layer is kept");
        assert_eq!(got.brand.unwrap().logo, None);
        assert_eq!(std::fs::read_dir(&assets).unwrap().count(), 0, "nothing written");
        for d in [src, tpl, assets] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }
}
