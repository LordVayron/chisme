//! Virtual background: the setting, and the worker that turns camera
//! frames into person masks off the capture thread.
//!
//! The camera thread hands over a 256x256 copy of a frame whenever the
//! worker is free (`segment::downsample`); the worker runs the model,
//! smooths the mask over time and publishes it. The engine uploads the
//! newest mask and `win::matte` blends camera and background on the GPU.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::segment::{SIZE, Segmenter, smooth};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Off,
    Blur,
    Image,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VirtualBackground {
    pub mode: Mode,
    /// Blur strength, 0..1.
    pub blur: f32,
    /// The picture used in Image mode.
    pub image: Option<String>,
    /// Pictures added before, most recent first.
    pub images: Vec<String>,
}

impl Default for VirtualBackground {
    fn default() -> Self {
        VirtualBackground { mode: Mode::Off, blur: 0.6, image: None, images: vec![] }
    }
}

impl VirtualBackground {
    pub fn active(&self) -> bool {
        match self.mode {
            Mode::Off => false,
            Mode::Blur => true,
            Mode::Image => self.image.is_some(),
        }
    }

    /// Use `path` (and remember it at the front of the list).
    pub fn pick(&mut self, path: String) {
        self.images.retain(|p| *p != path);
        self.images.insert(0, path.clone());
        self.images.truncate(8);
        self.image = Some(path);
        self.mode = Mode::Image;
    }
}

/// The newest mask: 256x256 bytes and a counter that changes with it.
#[derive(Default)]
pub struct Mask {
    pub seq: u64,
    pub data: Vec<u8>,
    /// Milliseconds the model took (last run).
    pub ms: f32,
}

pub struct Worker {
    /// Frames in (the camera sends one when this has room).
    pub input: SyncSender<Vec<f32>>,
    pub mask: Arc<Mutex<Mask>>,
    stop: Arc<AtomicBool>,
}

impl Worker {
    pub fn start() -> anyhow::Result<Worker> {
        let seg = Segmenter::new()?;
        let (tx, rx) = sync_channel::<Vec<f32>>(1);
        let mask = Arc::new(Mutex::new(Mask::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (m2, s2) = (mask.clone(), stop.clone());
        std::thread::Builder::new().name("segment".into()).spawn(move || run(seg, rx, m2, s2))?;
        Ok(Worker { input: tx, mask, stop })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(seg: Segmenter, rx: Receiver<Vec<f32>>, out: Arc<Mutex<Mask>>, stop: Arc<AtomicBool>) {
    let mut acc = vec![0f32; SIZE * SIZE];
    let mut first = true;
    // MODNet beside it, at its own pace: where the person can be. It ends
    // when this thread drops its sender.
    let allow: Arc<Mutex<Option<Vec<f32>>>> = Arc::default();
    let (mtx, mrx) = sync_channel::<Vec<f32>>(1);
    {
        let allow = allow.clone();
        let _ = std::thread::Builder::new().name("modnet".into()).spawn(move || {
            let Ok(path) = crate::segment::ensure_modnet() else { return };
            let Ok(net) = crate::segment::ModNet::new(&path) else { return };
            while let Ok(input) = mrx.recv() {
                if let Ok(a) = net.run(&input) {
                    *allow.lock().unwrap() = Some(crate::segment::allow_mask(&a, 8));
                }
            }
        });
    }
    // Ends when the camera side drops the sender.
    while let Ok(input) = rx.recv() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let t = std::time::Instant::now();
        let _ = mtx.try_send(input.clone());
        let Ok(mut m) = seg.run_input(input) else { continue };
        if let Some(a) = allow.lock().unwrap().as_ref() {
            for (v, k) in m.iter_mut().zip(a) {
                *v *= k;
            }
        }
        if first {
            acc.copy_from_slice(&m);
            first = false;
        } else {
            smooth(&mut acc, &m);
        }
        let data = acc.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8).collect();
        let mut o = out.lock().unwrap();
        o.data = data;
        o.seq += 1;
        o.ms = t.elapsed().as_secs_f32() * 1000.0;
    }
}
