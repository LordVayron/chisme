# Changelog

All notable changes to Chisme. The format follows [Keep a Changelog](https://keepachangelog.com),
and versions follow [Semantic Versioning](https://semver.org).

## [Unreleased]

### Fixed
- The command line shows "Transcribing" in English, and `chisme marker` names the right chapters file.

### Changed
- Updated eframe to 0.37, global-hotkey to 0.9 and tract-onnx to 0.23.9.

## [0.4.2] - 2026-10-19

### Fixed
- Garbled Spanish words in the app ("Cámara", "Capítulo") and in layer names.

## [0.4.1] - 2026-10-19

### Changed
- The guest page (green room) now has Chisme's look: the black background with the warm glow, Krona One titles with the rainbow bar, glass cards, a live mic meter, a name tag on the preview, and a PREVIEW / GREEN ROOM / LIVE badge. On phones the status shows first.

### Added
- A live end-to-end guest test (`cargo test -- --ignored live_room_end_to_end`) against a real LiveKit project.

## [0.4.0] - 2026-10-18

### Added
- **Guests.** Invite people into your show with a link: they open it in their browser, type their name, pick camera and mic, and wait in the green room. Chisme shows each guest's camera and mic level; bring them on stage, mute or remove them. On-stage guests are in the recording and the streams. Guests hear you and see the show. Runs on your own LiveKit Cloud project (free tier), with links signed by Chisme.
- **Guest scenes:** Interview, Panel (up to three guests), Guest full screen, and Screen + guests, in a "Guests" template group, ready to pin. A "+ Guest" layer in the scene editor.
- **Instagram Live** as a streaming destination (vertical 1080x1920).
- **New sound pad:** 18 studio-quality sounds (applause, laugh, ooh, drum roll, rimshot, ta-da, suspense, correct, buzzer, air horn, ding, bell, cash register, whoosh, glitch, record scratch, boing, crickets), replacing the old synthesized ones.

### Fixed
- Vertical streams (TikTok, Instagram) are now exactly 1080x1920.
- The command line now shows stream errors and English status words.
- The sidebar fits in short windows.
## [0.3.1] - 2026-10-18

### Added
- A starter sound pad on the Media Board: applause, drum roll, ding, whoosh, buzzer and ba-dum-tss, made by Chisme itself.

### Fixed
- Pinned scenes and templates now have their own row under the preview, with their names, and camera scenes stay visible when the camera isn't connected.

## [0.3.0] - 2026-10-18

### Added
- **Camera controls** in the Studio: zoom, pan/tilt pad, focus, exposure, brightness, contrast, saturation, sharpness, white balance and gain (whatever the camera's driver offers), with Auto and Reset.
- **Media Board in the Studio**, in place of the recordings list.
- **Teleprompter button** in the Studio.
- **Custom scenes:** a background picture, any border color, and a Save window that asks for the name and group.
- **Groups** for templates and Media Board items (right-click to move one to a group).
- **Recordings:** select one, several or all, and send them to the Recycle Bin.
- **Responsive layout:** every page adapts to small windows (down to 720x540).

### Changed
- The Layers menu only appears while building a custom scene, without the template chips; the add buttons sit at the top.
- The stats under the preview are a single quiet line.

### Fixed
- Deleting layers and moving them forward or back didn't work.

## [0.2.1] - 2026-10-18

### Added
- A "Show Chisme in recording" switch on the preview (and in Settings), to record tutorials of Chisme itself.
- Volume and mute for each audio source, plus "Mute all", right on the Live audio card. They apply live, even mid-recording.
- Quick access under the preview: pin and order scenes and templates from the Templates page.

## [0.2.0] - 2026-10-14

### Added
- **Virtual background:** blur or replace what's behind you on camera, with no green screen.
  MediaPipe Selfie Segmentation runs on your PC (`--bg-blur`, `--bg-image`).
- **Media Board:** fire sounds, clips and images with a click or Ctrl+Alt+1…9. They play on your
  speakers and land in recordings and streams.
- **Teleprompter:** a see-through, always-on-top script window that never shows in a recording.
- **Custom layouts:** drag and resize screen, camera, image and lower-third layers.
- **Brand kit:** a logo, two colors and a font, used by lower thirds and layouts.
- **Templates:** save layouts with their brand, share them as `.chisme` files and switch live
  with Ctrl+Shift+1…9.
- **Language:** English (the new default) or Spanish.
- **Shortcuts:** every global shortcut can be changed, and conflicts with other apps are shown.
- More scenes, chapter markers, vertical 9:16 export, per-app audio and RNNoise.

### Changed
- The interface draws with Direct3D 12 through DirectComposition (it used OpenGL, which some
  drivers fail to present).
- Chapter files are now named `<video>.chapters.txt`. The old `.capitulos.txt` files still count.

### Security
- Imported templates can only use the pictures they carry. They can't point Chisme at other files
  or at network shares, and only real PNG/JPEG data is written.
- Whisper models are checked against a pinned SHA-256 after downloading.
- Frames from cameras and video files are copied with bounds checks.

## 0.1.0 - 2026-10-13 (not released)

### Added
- First release. Screen, window, audio and webcam recording, live streaming to YouTube, Twitch,
  TikTok, X and RTMP, a replay buffer, Whisper subtitles, a tray icon, global hotkeys, a command
  line and the installer.

[Unreleased]: https://github.com/LordVayron/chisme/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/LordVayron/chisme/releases/tag/v0.4.2
[0.4.1]: https://github.com/LordVayron/chisme/releases/tag/v0.4.1
[0.4.0]: https://github.com/LordVayron/chisme/releases/tag/v0.4.0
[0.3.1]: https://github.com/LordVayron/chisme/releases/tag/v0.3.1
[0.3.0]: https://github.com/LordVayron/chisme/releases/tag/v0.3.0
[0.2.1]: https://github.com/LordVayron/chisme/releases/tag/v0.2.1
[0.2.0]: https://github.com/LordVayron/chisme/releases/tag/v0.2.0
