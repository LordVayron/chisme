//! Interface language: English (default) or Spanish. Strings live next to
//! the code that shows them, as pairs: `l("Record", "Grabar")`, or with
//! arguments, `t!("Saved: {}", "Guardado: {}", name)`. The engine thread
//! reads the same switch for its messages.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lang {
    #[default]
    En,
    Es,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Es];

    /// The language's own name.
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
        }
    }
}

static SPANISH: AtomicBool = AtomicBool::new(false);

pub fn set(lang: Lang) {
    SPANISH.store(lang == Lang::Es, Ordering::Relaxed);
}

pub fn spanish() -> bool {
    SPANISH.load(Ordering::Relaxed)
}

/// The English or the Spanish text.
pub fn l(en: &'static str, es: &'static str) -> &'static str {
    if spanish() { es } else { en }
}

/// `format!` with an English and a Spanish pattern (same arguments).
#[macro_export]
macro_rules! t {
    ($en:literal, $es:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::spanish() { format!($es $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches() {
        set(Lang::Es);
        assert_eq!(l("Record", "Grabar"), "Grabar");
        assert_eq!(t!("{} files", "{} archivos", 3), "3 archivos");
        set(Lang::En);
        assert_eq!(l("Record", "Grabar"), "Record");
        assert_eq!(t!("{} files", "{} archivos", 3), "3 files");
        assert_eq!(serde_json::from_str::<Lang>("\"es\"").unwrap(), Lang::Es);
    }
}
