# Changelog

All notable changes to Chisme. The format follows [Keep a Changelog](https://keepachangelog.com),
and versions follow [Semantic Versioning](https://semver.org).

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

## [0.1.0] - 2026-10-13

### Added
- First release. Screen, window, audio and webcam recording, live streaming to YouTube, Twitch,
  TikTok, X and RTMP, a replay buffer, Whisper subtitles, a tray icon, global hotkeys, a command
  line and the installer.

[0.2.0]: https://github.com/LordVayron/chisme/releases/tag/v0.2.0
[0.1.0]: https://github.com/LordVayron/chisme/releases/tag/v0.1.0
