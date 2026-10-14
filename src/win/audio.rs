//! System audio (WASAPI loopback) and microphone capture. Each source runs on its own
//! thread and sends stereo f32 packets stamped on the QPC host clock.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantToStringAlloc};
use windows::Win32::System::Com::{BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, IAgileObject, IAgileObject_Impl, STGM_READ};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{GUID, Interface, implement};

use crate::mixer::AudioKind;

pub struct Packet {
    pub kind: AudioKind,
    /// Host time of the first frame (seconds, QPC clock).
    pub pts: f64,
    pub rate: f64,
    pub frames: Vec<[f32; 2]>,
}

pub struct Device {
    pub name: String,
}

/// What to capture.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// Everything the default output plays.
    System,
    /// A microphone: None or "default" = the default one, else a name part.
    Mic(Option<String>),
    /// One app's audio (`include`), or everything except it (Windows 10
    /// 2004+ process loopback). `name`: part of its executable name.
    App { name: String, include: bool },
}

impl Source {
    pub fn kind(&self) -> AudioKind {
        if matches!(self, Source::Mic(_)) { AudioKind::Mic } else { AudioKind::System }
    }
}

pub(crate) fn com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Signals that ActivateAudioInterfaceAsync finished. It must be agile:
/// Windows calls it from its own thread.
#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Activated(std::sync::mpsc::SyncSender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(&self, _op: windows::core::Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

impl IAgileObject_Impl for Activated_Impl {}

/// An audio client capturing process `pid` and its children (`include`), or
/// everything except them.
fn process_client(pid: u32, include: bool) -> Result<IAudioClient> {
    unsafe {
        let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: if include { PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE } else { PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE },
                },
            },
        };
        // ManuallyDrop: dropping a PROPVARIANT frees its blob, which here
        // points at `params` on the stack.
        let mut pv = std::mem::ManuallyDrop::new(PROPVARIANT::default());
        let inner = &mut *pv.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB { cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &mut params as *mut _ as *mut u8 };
        let (tx, rx) = sync_channel(1);
        let handler: IActivateAudioInterfaceCompletionHandler = Activated(tx).into();
        let op = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*pv as *const PROPVARIANT), &handler)?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("Windows didn't open the app's audio"))?;
        let (mut hr, mut unk) = (windows::core::HRESULT(0), None);
        op.GetActivateResult(&mut hr, &mut unk)?;
        hr.ok().context("per-app audio needs Windows 10 2004 or later")?;
        Ok(unk.context("no audio client")?.cast()?)
    }
}

pub(crate) fn enumerator() -> Result<IMMDeviceEnumerator> {
    Ok(unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? })
}

fn friendly_name(dev: &IMMDevice) -> String {
    unsafe {
        dev.OpenPropertyStore(STGM_READ)
            .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
            .and_then(|v| PropVariantToStringAlloc(&v))
            .map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as _));
                s
            })
            .unwrap_or_else(|_| "Unknown device".into())
    }
}

fn endpoints(flow: EDataFlow) -> Result<Vec<IMMDevice>> {
    unsafe {
        let list = enumerator()?.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        (0..list.GetCount()?).map(|i| Ok(list.Item(i)?)).collect()
    }
}

/// Active microphones.
pub fn microphones() -> Result<Vec<Device>> {
    com();
    Ok(endpoints(eCapture)?.iter().map(|d| Device { name: friendly_name(d) }).collect())
}

/// The default output device's name (what system audio records).
pub fn default_output() -> Result<String> {
    com();
    Ok(friendly_name(&unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole)? }))
}

/// `None` or "default": the default mic; else a name substring.
fn find_mic(query: Option<&str>) -> Result<IMMDevice> {
    match query.filter(|q| !q.is_empty() && !q.eq_ignore_ascii_case("default")) {
        None => unsafe { enumerator()?.GetDefaultAudioEndpoint(eCapture, eConsole) }.context("no microphone"),
        Some(q) => {
            let ql = q.to_lowercase();
            endpoints(eCapture)?
                .into_iter()
                .find(|d| friendly_name(d).to_lowercase().contains(&ql))
                .ok_or_else(|| anyhow!("no microphone matches {q:?}; see `chisme devices`"))
        }
    }
}

/// How to read the device's mix format.
#[derive(Clone, Copy)]
struct Format {
    rate: f64,
    channels: usize,
    float: bool,
    bits: u16,
}

// KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
const SUBTYPE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

unsafe fn format(p: *const WAVEFORMATEX) -> Format {
    let f = unsafe { std::ptr::read_unaligned(p) };
    let float = match f.wFormatTag as u32 {
        3 => true, // WAVE_FORMAT_IEEE_FLOAT
        0xFFFE => {
            let x = unsafe { std::ptr::read_unaligned(p as *const WAVEFORMATEXTENSIBLE) };
            let sub = x.SubFormat;
            sub == SUBTYPE_FLOAT
        }
        _ => false,
    };
    Format { rate: f.nSamplesPerSec as f64, channels: f.nChannels.max(1) as usize, float, bits: f.wBitsPerSample }
}

/// Interleaved device samples to stereo (mono is duplicated; extra channels
/// beyond front left/right are dropped).
fn to_stereo(fmt: Format, data: *const u8, frames: usize) -> Vec<[f32; 2]> {
    let ch = fmt.channels;
    let read = |i: usize| -> f32 {
        unsafe {
            match (fmt.float, fmt.bits) {
                (true, 32) => std::ptr::read_unaligned((data as *const f32).add(i)),
                (false, 16) => std::ptr::read_unaligned((data as *const i16).add(i)) as f32 / 32768.0,
                (false, 32) => std::ptr::read_unaligned((data as *const i32).add(i)) as f32 / 2147483648.0,
                (false, 24) => {
                    let p = data.add(i * 3);
                    let v = ((*p as i32) | ((*p.add(1) as i32) << 8) | ((*p.add(2) as i32) << 16)) << 8 >> 8;
                    v as f32 / 8388608.0
                }
                _ => 0.0,
            }
        }
    };
    (0..frames)
        .map(|f| {
            let l = read(f * ch);
            [l, if ch > 1 { read(f * ch + 1) } else { l }]
        })
        .collect()
}

pub struct AudioCapture {
    pub name: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Start capturing `kind` (`mic` selects the microphone). Packets go to `tx`.
    /// Start capturing `source`. Packets go to `tx`.
    pub fn start(source: Source, tx: Sender<Packet>) -> Result<AudioCapture> {
        let kind = source.kind();
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = sync_channel::<Result<String>>(1);
        let stop2 = stop.clone();
        let thread = std::thread::Builder::new().name(format!("audio-{kind:?}")).spawn(move || {
            com();
            let opened = (|| -> Result<_> {
                unsafe {
                    let (name, client, fmt) = match &source {
                        Source::App { name, include } => {
                            let app = super::apps::find(name)?;
                            let client = process_client(app.pid, *include)?;
                            // Process loopback has no mix format: ask for 48 kHz
                            // stereo 16-bit and let Windows convert.
                            let wfx = WAVEFORMATEX {
                                wFormatTag: 1, // PCM
                                nChannels: 2,
                                nSamplesPerSec: 48000,
                                nAvgBytesPerSec: 48000 * 4,
                                nBlockAlign: 4,
                                wBitsPerSample: 16,
                                cbSize: 0,
                            };
                            let flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
                            client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &wfx, None).context("per-app audio needs Windows 10 2004 or later")?;
                            let label = if *include { format!("solo {}", app.name) } else { format!("todo excepto {}", app.name) };
                            (label, client, Format { rate: 48000.0, channels: 2, float: false, bits: 16 })
                        }
                        _ => {
                            let dev = match &source {
                                Source::Mic(q) => find_mic(q.as_deref())?,
                                _ => enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole).context("no output device")?,
                            };
                            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
                            let mix = client.GetMixFormat()?;
                            let fmt = format(mix);
                            let flags = if kind == AudioKind::System { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
                            // 200 ms device buffer; read every 10 ms.
                            let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, mix, None);
                            CoTaskMemFree(Some(mix as _));
                            init?;
                            (friendly_name(&dev), client, fmt)
                        }
                    };
                    if !(fmt.float && fmt.bits == 32) && !matches!(fmt.bits, 16 | 24 | 32) {
                        bail!("unsupported audio format ({} bit)", fmt.bits);
                    }
                    let cap: IAudioCaptureClient = client.GetService()?;
                    client.Start()?;
                    Ok((name, client, cap, fmt))
                }
            })();
            let (client, cap, fmt) = match opened {
                Ok((name, client, cap, fmt)) => {
                    let _ = ready_tx.send(Ok(name));
                    (client, cap, fmt)
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                loop {
                    let n = unsafe { cap.GetNextPacketSize() }.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    let (mut data, mut frames, mut flags, mut qpc) = (std::ptr::null_mut(), 0u32, 0u32, 0u64);
                    if unsafe { cap.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc)) }.is_err() {
                        break;
                    }
                    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                    let out = if silent { vec![[0.0, 0.0]; frames as usize] } else { to_stereo(fmt, data, frames as usize) };
                    unsafe {
                        let _ = cap.ReleaseBuffer(frames);
                    }
                    // Process loopback may not stamp packets: use the clock.
                    let pts = if qpc > 0 { qpc as f64 * 1e-7 } else { super::now() - frames as f64 / fmt.rate };
                    if frames > 0 && tx.send(Packet { kind, pts, rate: fmt.rate, frames: out }).is_err() {
                        return;
                    }
                }
            }
            unsafe {
                let _ = client.Stop();
            }
        })?;
        let name = ready_rx.recv().unwrap_or_else(|_| Err(anyhow!("audio thread ended")))?;
        Ok(AudioCapture { name, stop, thread: Some(thread) })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
