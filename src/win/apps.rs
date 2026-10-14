//! Apps for per-app audio capture: processes by name, and the apps that
//! currently have an audio session on the default output.

use anyhow::{Result, anyhow};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{IAudioSessionControl2, IAudioSessionManager2, eConsole, eRender};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows::core::Interface;

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    /// Executable name without ".exe" (e.g. "Discord").
    pub name: String,
}

/// Every running process.
pub fn processes() -> Vec<Proc> {
    let mut out = vec![];
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let exe = String::from_utf16_lossy(&e.szExeFile[..n]);
            let name = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(&exe).to_string();
            out.push(Proc { pid: e.th32ProcessID, parent: e.th32ParentProcessID, name });
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// The top process of an app (Discord, Chrome and games run many): the
/// matching process whose parent isn't the same app. Capturing its process
/// tree then covers all of them.
pub fn root_of(procs: &[Proc], query: &str) -> Option<Proc> {
    let q = query.trim().trim_end_matches(".exe").to_lowercase();
    let matches: Vec<&Proc> = procs.iter().filter(|p| p.name.to_lowercase() == q).collect();
    let matches = if matches.is_empty() { procs.iter().filter(|p| p.name.to_lowercase().contains(&q)).collect() } else { matches };
    let same = |pid: u32, name: &str| procs.iter().any(|p| p.pid == pid && p.name.eq_ignore_ascii_case(name));
    matches.iter().find(|p| !same(p.parent, &p.name)).or(matches.first()).map(|p| (*p).clone())
}

/// Find a running app by (part of) its executable name.
pub fn find(query: &str) -> Result<Proc> {
    root_of(&processes(), query).ok_or_else(|| anyhow!("\"{query}\" no está abierto"))
}

/// Apps with an audio session on the default output (playing or ready to),
/// by name, without duplicates, Chisme itself left out.
pub fn audio_apps() -> Result<Vec<String>> {
    super::audio::com();
    let procs = processes();
    let mut names: Vec<String> = vec![];
    unsafe {
        let dev = super::audio::enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None)?;
        let sessions = mgr.GetSessionEnumerator()?;
        for i in 0..sessions.GetCount()? {
            let Ok(c) = sessions.GetSession(i).and_then(|s| s.cast::<IAudioSessionControl2>()) else { continue };
            let Ok(pid) = c.GetProcessId() else { continue };
            if pid == 0 || pid == std::process::id() {
                continue; // system sounds, Chisme
            }
            // Name the app by its root process (a Chrome tab → "chrome").
            let Some(p) = procs.iter().find(|p| p.pid == pid) else { continue };
            let root = root_of(&procs, &p.name).map_or(p.name.clone(), |r| r.name);
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&root)) && !root.eq_ignore_ascii_case("chisme") && !root.eq_ignore_ascii_case("chisme-app") {
                names.push(root);
            }
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, name: &str) -> Proc {
        Proc { pid, parent, name: name.into() }
    }

    #[test]
    fn picks_the_app_root() {
        let procs = vec![p(4, 1, "explorer"), p(10, 4, "Discord"), p(11, 10, "Discord"), p(12, 10, "Discord"), p(20, 4, "chrome"), p(21, 20, "chrome")];
        assert_eq!(root_of(&procs, "discord").unwrap().pid, 10);
        assert_eq!(root_of(&procs, "Discord.exe").unwrap().pid, 10);
        assert_eq!(root_of(&procs, "chro").unwrap().pid, 20, "a name part works too");
        assert!(root_of(&procs, "spotify").is_none());
    }

    #[test]
    fn lists_real_processes() {
        let me = std::process::id();
        assert!(processes().iter().any(|p| p.pid == me));
    }
}
