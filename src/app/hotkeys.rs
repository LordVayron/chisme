//! Global shortcuts, set in Settings → Shortcuts. Each action has a combo
//! like "Ctrl+Shift+R" (empty: off); templates and Media Board tiles share
//! a modifier set followed by 1…9. Combos another app holds are reported.

use eframe::egui;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;

use crate::i18n::l;
use crate::settings::Hotkeys;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Record,
    Pause,
    Marker,
    Replay,
    Prompter,
    PrompterPlay,
    /// Apply template 1…9 (0-based).
    Template(usize),
    /// Fire Media Board tile 1…9 (0-based).
    Media(usize),
}

impl Action {
    /// The actions with a combo of their own.
    pub const SINGLE: [Action; 6] = [Action::Record, Action::Pause, Action::Marker, Action::Replay, Action::Prompter, Action::PrompterPlay];

    pub fn label(self) -> String {
        match self {
            Action::Record => l("Record / stop", "Grabar / detener").into(),
            Action::Pause => l("Pause / resume", "Pausar / reanudar").into(),
            Action::Marker => l("Mark a chapter", "Marcar capítulo").into(),
            Action::Replay => l("Save replay", "Guardar replay").into(),
            Action::Prompter => l("Teleprompter on / off", "Teleprompter sí / no").into(),
            Action::PrompterPlay => l("Teleprompter play / pause", "Teleprompter play / pausa").into(),
            Action::Template(i) => crate::t!("Template {}", "Plantilla {}", i + 1),
            Action::Media(i) => crate::t!("Media Board tile {}", "Media Board, tarjeta {}", i + 1),
        }
    }

    /// The combo for this action ("" when off).
    pub fn combo(self, h: &Hotkeys) -> String {
        let digit = |mods: &str, i: usize| if mods.is_empty() { String::new() } else { format!("{mods}+{}", i + 1) };
        match self {
            Action::Template(i) => digit(&h.templates, i),
            Action::Media(i) => digit(&h.media, i),
            a => a.slot(&mut h.clone()).clone(),
        }
    }

    /// The setting holding a single action's combo.
    pub fn slot(self, h: &mut Hotkeys) -> &mut String {
        match self {
            Action::Record => &mut h.record,
            Action::Pause => &mut h.pause,
            Action::Marker => &mut h.marker,
            Action::Replay => &mut h.replay,
            Action::Prompter => &mut h.prompter,
            Action::PrompterPlay => &mut h.prompter_play,
            Action::Template(_) => &mut h.templates,
            Action::Media(_) => &mut h.media,
        }
    }

    /// Every action, the numbered ones included.
    pub fn all() -> Vec<Action> {
        let mut v = Action::SINGLE.to_vec();
        v.extend((0..9).map(Action::Template));
        v.extend((0..9).map(Action::Media));
        v
    }
}

/// The modifier sets offered for the numbered shortcuts ("" = off).
pub const GROUP_MODS: [&str; 5] = ["Ctrl+Shift", "Ctrl+Alt", "Alt+Shift", "Ctrl+Alt+Shift", ""];

/// A pressed key as a combo, if it makes a usable global shortcut: at least
/// one modifier (or a function key), and a key Windows can register.
pub fn from_egui(key: egui::Key, m: egui::Modifiers) -> Option<String> {
    let mut s = String::new();
    if m.ctrl || m.command {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    let name = key.name();
    let function_key = name.len() >= 2 && name.starts_with('F') && name[1..].chars().all(|c| c.is_ascii_digit());
    if s.is_empty() && !function_key {
        return None;
    }
    s.push_str(name);
    s.parse::<HotKey>().ok().map(|_| s)
}

/// The OS registrations, kept in step with the settings.
pub struct Registry {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<(Action, HotKey)>,
    /// The settings last applied.
    pub applied: Option<Hotkeys>,
    /// Actions whose combo another app holds.
    pub taken: Vec<Action>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry { manager: GlobalHotKeyManager::new().ok(), registered: vec![], applied: None, taken: vec![] }
    }

    /// Drop every registration (while a new combo is being typed).
    pub fn clear(&mut self) {
        if let Some(m) = &self.manager {
            for (_, k) in self.registered.drain(..) {
                let _ = m.unregister(k);
            }
        }
        self.applied = None;
    }

    /// Register the combos in `h` (replacing the previous ones).
    pub fn apply(&mut self, h: &Hotkeys) {
        self.clear();
        self.taken.clear();
        let Some(m) = &self.manager else { return };
        for a in Action::all() {
            let combo = a.combo(h);
            let Ok(k) = combo.parse::<HotKey>() else { continue };
            if self.registered.iter().any(|(_, r)| r.id() == k.id()) {
                continue; // the same combo twice: the first action keeps it
            }
            match m.register(k) {
                Ok(()) => self.registered.push((a, k)),
                Err(_) => self.taken.push(a),
            }
        }
        self.applied = Some(h.clone());
    }

    pub fn action(&self, id: u32) -> Option<Action> {
        self.registered.iter().find(|(_, k)| k.id() == id).map(|(a, _)| *a)
    }
}

/// The action already using `combo`, other than `except`.
pub fn used_by(h: &Hotkeys, combo: &str, except: Action) -> Option<Action> {
    let k: HotKey = combo.parse().ok()?;
    Action::all().into_iter().filter(|a| *a != except).find(|a| a.combo(h).parse::<HotKey>().is_ok_and(|o| o.id() == k.id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos() {
        let h = Hotkeys::default();
        assert_eq!(Action::Record.combo(&h), "Ctrl+Shift+R");
        assert_eq!(Action::Template(2).combo(&h), "Ctrl+Shift+3");
        assert_eq!(Action::Media(0).combo(&h), "Ctrl+Alt+1");
        assert!(Action::all().iter().all(|a| a.combo(&h).parse::<HotKey>().is_ok()), "the defaults all parse");
        assert_eq!(used_by(&h, "ctrl+shift+r", Action::Pause), Some(Action::Record));
        assert_eq!(used_by(&h, "Ctrl+Shift+R", Action::Record), None);
        assert_eq!(used_by(&h, "Ctrl+Alt+2", Action::Record), Some(Action::Media(1)));
        let off = Hotkeys { media: String::new(), ..Default::default() };
        assert_eq!(Action::Media(0).combo(&off), "");
    }

    #[test]
    fn keys_from_egui() {
        let ctrl_shift = egui::Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(from_egui(egui::Key::K, ctrl_shift).as_deref(), Some("Ctrl+Shift+K"));
        assert_eq!(from_egui(egui::Key::F9, egui::Modifiers::NONE).as_deref(), Some("F9"));
        assert_eq!(from_egui(egui::Key::K, egui::Modifiers::NONE), None, "a bare letter would block typing");
        assert_eq!(from_egui(egui::Key::Num4, egui::Modifiers { alt: true, ..Default::default() }).as_deref(), Some("Alt+4"));
    }
}
