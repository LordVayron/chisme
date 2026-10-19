# Chisme: the technical guide

How every part of Chisme works, and how to drive it from the command line. For an overview, see
the [README](../README.md).

## The app

Run `chisme-app.exe` (or `chisme.exe` with no arguments).

- **Design:** Ventura's black, wide type (Krona One) and rainbow stripe, laid out with Channel
  Analytics' glass cards and icon sidebar.
- **Studio:**
  - A live preview with a camera overlay you can drag; it snaps to corners.
  - A white Record button, pause, and a scene picker.
  - A stats row: duration, fps, bitrate, dropped frames.
  - A live audio chart.
  - Camera controls: shape, size, roundness, border and color, shadow, mirror, corner.
  - Your recordings.
- **Recordings** lists every MP4 with Open and Show in folder.
- **Settings:**
  - Source: a display or a window.
  - Cursor and camera.
  - Resolution, fps and codec.
  - System audio, mic and track layout.
  - Output folder.
  - These are saved in `%APPDATA%\Chisme\settings.json`.
- **Ctrl+Shift+R** records and stops, **Ctrl+Shift+P** pauses. Both work while the window is
  hidden.
- **Tray icon:**
  - Its menu records, pauses, shows the window, opens the folder and quits.
  - Closing the window while recording keeps it recording in the tray.
- **Chisme's own window is never captured.** Set `CHISME_SHOW_IN_CAPTURE=1` to
  allow it (for screenshots of the app).

## Streaming and replay buffer

The **Live** page streams to YouTube, Twitch, TikTok, Instagram, X or any RTMP(S) server, all
at once.

**How it works:**
- Each destination gets its own hardware H.264 encode (an async Media Foundation MFT on
  NVENC/AMF/QSV) plus AAC. This lets TikTok get a vertical 9:16 crop at 30 fps while YouTube gets
  1080p60 landscape.
- The RTMP client handles AMF0, chunking, FLV, GOP dropping under congestion, and reconnects
  with backoff. RTMPS goes through Windows' SChannel.
- Stream keys live in Windows Credential Manager, never in `settings.json`.

| Platform | Server URL | Encode |
|---|---|---|
| YouTube | built in | 1080p, up to 60 fps, 12 Mb/s |
| Twitch | built in | 1080p, up to 60 fps, 6 Mb/s |
| TikTok | from LIVE Studio (changes each time) | vertical 1080×1920, 30 fps, 4.5 Mb/s, AAC 128 |
| Instagram | from instagram.com → Create → Live video → Streaming software | vertical 1080×1920, 30 fps, 4.5 Mb/s, keyframe 2 s, AAC 128 |
| X | from Media Studio → Producer | 1080p, 30 fps, 9 Mb/s, keyframe 3 s, AAC 128 |
| Custom | any rtmp:// or rtmps:// | 1080p, up to 60 fps, 6 Mb/s |

The **replay buffer** keeps the last 15–300 s in memory. **Ctrl+Shift+S** (or the button, the
tray menu, or `chisme save-replay`) saves them to `Videos\Chisme\Chisme replay <date>.mp4`.

```powershell
chisme stream-key youtube                     # paste the key; stored in Credential Manager
chisme record --stream youtube                # record and stream
chisme record --no-record --stream twitch --stream "tiktok=rtmp://…/game" --replay 60
chisme save-replay                            # from another terminal
```

## Guests

The **Guests** page brings people into your show from a link. There's nothing for them to
install.

1. Create a free project at [cloud.livekit.io](https://cloud.livekit.io), open
   **Settings → API keys**, and paste the URL, key and secret into Chisme. The secret goes to
   Credential Manager. The free tier covers about 80 hours of guest time a month.
2. Click **Open green room**, then **Copy invite link** once per guest. Each link lasts 24 hours.
3. The guest opens the link, types a name, picks a camera and mic, and joins. They wait there,
   hearing you and seeing the show.
4. In Chisme, each guest shows up with their camera and mic level. **On stage** puts them in the
   recording and streams. **Mute** silences them. **Remove** ends their session.

The guest scenes (**Interview**, **Panel**, **Guest full screen**, **Screen + guests**) place
the people on stage in the order you brought them on. You can pin them to quick access, or add
a **+ Guest** layer to any scene. The green room page is `docs/join/` in this repo, served by
GitHub Pages.

## Automatic subtitles (Whisper)

Chisme writes an `.srt` next to a recording with whisper.cpp (`whisper-rs`), entirely on this PC.
- In the app, use **CC** on any recording, or turn on *After every recording* under Settings → Subtitles to
  transcribe every recording when it ends.
- From the terminal: `chisme subtitles FILE [--model small] [--lang es]`.

How it works:
- The first audio track (the mix) is decoded with Media Foundation and resampled to 16 kHz mono.
- Whisper transcribes it with beam search, detecting the language, using one thread per
  physical core.
- Lines are cut at 48 characters, between words.
- The model downloads once to `%LOCALAPPDATA%\Chisme\models`:

| Model | Size |
|---|---|
| base | 60 MB |
| small (default) | 190 MB |
| medium | 540 MB |
| large-v3-turbo | 550 MB |

Speed: `small` transcribes about as fast as real time on a Ryzen 5 3600 (CPU, AVX2). Building it
needs:
- CMake (included in the VS Build Tools).
- LLVM's libclang (`winget install LLVM.LLVM`).

`.cargo/config.toml` sets AVX2 and `/O2` for MSVC. Without them whisper.cpp builds ~20× slower.

## More features

**Per-app audio:** record one app's sound, or everything except it. It uses Windows 10 2004+
process loopback and covers the app's whole process tree.
- In the app: Settings → Audio → *From which apps*. It lists the apps that have sound right now.
- From the terminal: `--app-audio discord` or `--exclude-app-audio spotify`.

**Mic noise suppression:** RNNoise (`nnnoiseless`, pure Rust) removes fans, hum and keyboard noise
from the mic.
- It reduces room-like noise by 40–50 dB in tests.
- It works on 10 ms blocks that keep their capture times, so A/V sync stays at 0 ms.
- On by default; `--no-denoise` turns it off.

**Chapter markers:** mark chapters while recording.
- Use **Ctrl+Shift+M**, the flag button, the tray menu or `chisme marker`.
- When recording ends, `<video>.chapters.txt` is written in YouTube's format (`00:00 Intro`,
  `01:23 Chapter 1`…).
- Paste it into the description to get chapters. Paused time doesn't count.

**Vertical clips 9:16:** a vertical copy (1080×1920) of any recording, for TikTok, Reels and
Shorts.
- Media Foundation decodes the file, the GPU crops the center 9:16, and the hardware encoder writes
  it.
- Use the **9:16** button on a recording, or *Save vertical 9:16 clip* in the replay card.
- From the terminal: `chisme vertical FILE`.
- 12 s exports in ~3.6 s, with A/V sync kept at 0 ms.

**More scenes:** five layouts, picked with the icons under the preview.

| Scene | Layout |
|---|---|
| Screen | Screen only |
| Screen + camera | Camera overlay on the screen |
| Side by side | Screen on the left, camera column on the right |
| Camera | Camera full frame |
| Camera + screen | Camera full frame, screen in a box you can drag |

**Custom layouts (layers):** the sixth scene icon, *Custom*, turns the current scene into
editable layers (`src/layout.rs`).
- **In the preview:**
  - Click a layer to select it, drag to move it, and drag a corner to resize it.
  - Shift keeps the proportions.
  - Orange guides snap to the frame's edges and center, and to other layers.
- **The Layers card:**
  - Add a screen or camera layer, show or hide it, change its order, delete it.
  - Set the background color.
  - Per layer: fill (crop) or fit, shape, roundness, border and its color, shadow, mirror.
- **How screen layers draw:** the video processor scales the screen to the layer's size first,
  which keeps text sharp. The same shader as the camera then adds the shape, border and shadow.
- **Saving:** layouts are saved in settings and change live while recording or streaming.
- **From the terminal:** `chisme record --layout file.json` uses a layout saved as JSON.

**Images, lower thirds and brand kit:**
- **Image layers:** PNG or JPG with transparency, for a logo, a frame or a full background. Fill
  or fit.
- **Text layers** are lower thirds: a name and a title in five styles.

| Style | Look |
|---|---|
| Simple | Text with a soft shadow |
| Block | Name on the primary color, title on the secondary |
| Pill | A white pill |
| Bar | A dark box with the rainbow accent |
| Display | Krona One in capitals with an underline |

- **How text draws:** it is rasterized on the CPU (`ab_glyph`, bundled fonts) at the layer's
  size and cached as a texture until it changes.
- **Settings → Brand:** the logo, primary and secondary colors, and a font (Inter or Krona One).
  - Lower thirds and the background color choices use the brand colors.
  - *Add the logo to the layout* places the logo in the top-right corner.

**Templates:** a template is a named layout plus the brand kit, saved in
`%APPDATA%\Chisme\plantillas\*.chisme` (JSON).
- **The Templates page:**
  - Cards with a drawn thumbnail, with Apply, Export and Delete.
  - "Save template" saves the current view.
  - "Import .chisme" adds a shared one.
- **Switching:** **Ctrl+Shift+1…9** (by default) switches template live, even while recording. The Layers card
  has numbered chips for the same.
- **Sharing:** exported files carry their images (logo, backgrounds) as base64. Importing keeps
  only real PNG/JPEG pictures, writes them to `%APPDATA%\Chisme\recursos` under names of its own,
  and drops any picture the file doesn't carry, so a shared template can't point Chisme at other
  files or network shares.
- **Starters:** three come from the scenes (Screen + camera, Side by side, Presenter).
- **From the terminal:** `chisme record --template "side by side"`.

**Teleprompter:** the *Script* page holds the script (word count and reading time) and the
teleprompter controls.
- **The window:** borderless, see-through, always on top and excluded from every capture. It
  opens at the top center of the main screen, under the webcam. Drag it from the background,
  resize it from the corner.
- **Reading:** the script scrolls past a reading line (orange arrows); lines already read fade.
  Play from the top starts with a 3-2-1.
- **Controls:** speed, text size, background opacity, reading line height and mirror (for a
  beam-splitter glass). They are saved in the settings.
- **Keys:** Ctrl+Shift+T (by default) opens or closes it, Ctrl+Shift+Space plays or pauses (both global).
  Inside it: Space or double-click plays, the wheel moves, arrows change speed, Home goes back
  to the top, Esc closes.
- **Rendering:** the app draws with Direct3D 12 presenting through DirectComposition, the
  only Windows swapchain with an alpha channel. Windows still paints the window's own surface
  white underneath, so the teleprompter is a layered window keyed on white.

**Media Board:** sounds (MP3, WAV, M4A, AAC, WMA, FLAC), clips (MP4, MOV, M4V, WMV, AVI) and
images (PNG, JPG) as tiles, added with a button or by dropping files on the window.
- **Firing:** a click or **Ctrl+Alt+1…9** (global, by default). Firing a sound or clip again stops it; an
  image stays over the scene until fired again. *Stop all* clears everything.
- **Picture:** a clip covers the scene (letterboxed on black) while it plays; an image is drawn
  on top with its transparency.
- **Sound:** Media Foundation decodes it to 48 kHz stereo (audio and picture have a reader each,
  so a file's interleaving never holds the sound back). You hear it on your speakers, timed to
  the recording. When system audio is recorded from the speakers it already hears it; otherwise
  (system audio off, or only one app's audio) it goes straight into the mix, the streams and
  the system stem (or the only track). Volume per tile, 0–200%.
- **Sync:** each stream starts at 0 (Media Foundation ignores the edit list of B-frame video),
  and playback starts once both have data. Measured: picture vs. sound within 5 ms.

**Virtual background:** blur what's behind you on camera, or replace it with a picture of your
own, without a green screen. Pick it under *Behind you* in the Camera card (or the Layers card),
or from the terminal with `--bg-blur [0..1]` / `--bg-image FILE`.
- **The model:** Google's MediaPipe Selfie Segmentation (Apache-2.0; the ONNX export from
  [onnx-community](https://huggingface.co/onnx-community/mediapipe_selfie_segmentation)), 460 KB,
  built into the program and run on the CPU with tract (pure Rust). About 30 ms per frame on its
  own thread; nothing leaves your PC.
- **Dim rooms:** the model's copy of the frame is auto-exposed (brightest 1% to white, shadows
  lifted); the video itself is untouched.
- **Blending:** on the GPU. The 256x256 mask is smoothed over time (fast where you move, slow at
  flickering edges), sampled bilinearly and tightened, so hair and shoulders stay solid. The blur
  is a 13-tap Gaussian on an 8x smaller copy of the camera; pictures are cropped to fill.
- **Pictures:** the last eight you used stay in the list.

**Language:** English (default) or Spanish, in Settings → General. Every screen, message and
tray item switches at once; the engine's messages follow too.

**Shortcuts:** Settings → Shortcuts. Click one and press the new keys (Esc cancels, Backspace
turns it off). Templates and Media Board tiles use a modifier set plus 1…9. Shortcuts another
app already holds are marked, and a combo used by two actions is refused. They're released
while you type a new one, so you can reuse a current combo.

**Background jobs:** subtitles and vertical exports run one at a time in a queue, with progress
shown in the recordings list.

## Installer and A/V sync

```powershell
powershell -ExecutionPolicy Bypass -File scripts\package.ps1   # → dist\Chisme-Setup-<version>.exe
```

**What the installer does:**
- Installs per user by default (no admin prompt), in Spanish or English.
- Adds a Start menu shortcut, plus an optional desktop shortcut.
- Can add `chisme` to the PATH.
- Includes an uninstaller that also removes the PATH entry.

**Two executables:**
- `chisme-app.exe` is the app, with no console window.
- `chisme.exe` is the command line. Run with no arguments, it also opens the app.

Both embed the icon (`src/icon.rs`, generated at build time) and a manifest with per-monitor DPI
awareness.

**A/V sync check.** The frame flashes white and the audio beeps at the
same instant every second; `synccheck.py` measures the offset in the file:

```powershell
chisme record --test-pattern -t 20 -o sync.mp4
python scripts\synccheck.py sync.mp4 --max-ms 5
```

Measured offset: **0.0 ms** at 60, 30 and 25 fps, with the webcam on and across a pause.

The executables aren't code-signed (that needs a paid certificate), so Windows SmartScreen warns
the first time; choose "More info → Run anyway".

One capture engine (`src/engine.rs`) keeps the screen, camera and audio running for the preview
and attaches the encoder when you press Record, so recording starts instantly. The CLI uses the
same engine.

## How it works

Video stays on the GPU from capture to file:

- **Windows.Graphics.Capture** captures a display or window, including the mouse pointer.
- The **D3D11 video processor** scales the frame. With a webcam it draws into a BGRA canvas,
  then converts to NV12 (BT.709, limited range).
- **The webcam overlay** is a D3D11 pixel shader. The camera comes in through the Media
  Foundation source reader. Shape, border and shadow masks are built once per size and style.
  Shapes are circle, rounded, square, pill and squircle.
- The **Media Foundation sink writer** encodes with your GPU's hardware encoder (NVENC, AMF or
  Quick Sync) into H.264 High or HEVC, plus AAC audio tracks (48 kHz stereo, 192 kb/s).
- **WASAPI** captures system audio (loopback of the default output) and the microphone.
  - The mixer puts both on the video's clock.
  - It resamples, limits at -1 dBFS, and fills silence when nothing plays.
  - With both sources there are three tracks: mix, system and mic.
- A **pacer** emits frames on an exact `1/fps` grid and repeats the last picture when the screen
  doesn't change, so editors see a true constant frame rate.
- **Pause and resume** cut whole video frames and the matching audio samples, leaving no gap in
  the file.

## Build

See [Building from source](../README.md#building-from-source) in the README.

## Use

```powershell
chisme devices                                  # displays, cameras, mics, windows
chisme record                                   # primary display + system audio + default mic
chisme record --mic maono --audio-tracks mix    # pick a mic, one mixed track
chisme record --cam                             # add the webcam in the bottom-right corner
chisme record --cam iriun --pip-shape circle --pip-border 8 --pip-border-color ff6a00 --pip-shadow
chisme record --cam --pip-pos 0.5,0.8 --pip-size 0.3
chisme record --scene cam --cam                 # webcam full frame
chisme record --window "Chrome" -t 60 --no-mic  # one window, 60 s, no microphone
chisme record -d 2 -r native -f 30 --codec hevc
chisme pause | chisme resume | chisme stop      # from another terminal (or Ctrl+C)
```

More options: `--no-system-audio`, `--no-mic`, `--system-gain`/`--mic-gain` (dB, mix track),
`--audio-tracks both|mix|stems`, `--pip-corner`, `--pip-roundness`, `--no-mirror`,
`--no-cursor`, `--bitrate`. Run `chisme record --help` for the full list.

## License

Copyright © 2026 Jorge (LordVayron) and 079 AI Labs. MIT OR Apache-2.0; third-party
attributions are in [NOTICE.md](../NOTICE.md).
