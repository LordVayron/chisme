//! `chisme stop|pause|resume` from another terminal, through named events.

use anyhow::{Result, bail};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, EVENT_MODIFY_STATE, OpenEventW, SetEvent, WaitForSingleObject};
use windows::core::HSTRING;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Stop,
    Pause,
    Resume,
    SaveReplay,
    Marker,
}

const ALL: [Command; 5] = [Command::Stop, Command::Pause, Command::Resume, Command::SaveReplay, Command::Marker];

fn name(c: Command) -> HSTRING {
    HSTRING::from(format!("Local\\Chisme.{c:?}"))
}

/// The recorder's side: one auto-reset event per command.
pub struct Listener {
    events: Vec<(Command, HANDLE)>,
}

impl Listener {
    pub fn new() -> Result<Listener> {
        let events = ALL
            .iter()
            .map(|&c| Ok((c, unsafe { CreateEventW(None, false, false, &name(c))? })))
            .collect::<Result<_>>()?;
        Ok(Listener { events })
    }

    /// Commands sent since the last poll.
    pub fn poll(&self) -> Vec<Command> {
        self.events
            .iter()
            .filter(|(_, h)| unsafe { WaitForSingleObject(*h, 0) } == WAIT_OBJECT_0)
            .map(|(c, _)| *c)
            .collect()
    }
}

/// Send a command to the running recorder.
pub fn send(c: Command) -> Result<()> {
    let Ok(h) = (unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &name(c)) }) else {
        bail!("Chisme isn't recording")
    };
    unsafe { SetEvent(h)? };
    Ok(())
}
