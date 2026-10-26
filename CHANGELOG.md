# Changelog

All notable changes to Chisme. The format follows [Keep a Changelog](https://keepachangelog.com),
and versions follow [Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.7.0] - 2026-10-26

### Added
- Tutorial mode: a new card on the Studio page with one main switch (off by default) and a switch per part. Each part can go to the recording, the stream or both, and it all works live:
  - Click effects: a ring on every click, in your brand color or any color you pick.
  - Bigger cursor (1.2x to 3x), hidden while the mouse rests.
  - Auto zoom: zooms the screen in where you click or type, follows the cursor near the edges, and zooms back out when you stop, with smooth motion and adjustable strength. Pause it with a shortcut (Ctrl+Alt+Z; shortcuts change in Settings → Shortcuts) or its Pause switch when you don't want every click to zoom; the rest keeps working. Hold it with Ctrl+Alt+H (or its Hold switch) to keep the zoom still where it is, or zoom in at the cursor, so people can read; press again to zoom back out.
  - Smooth cursor: Chisme draws the cursor itself, gliding instead of jumping.
  - Shortcut banner: shows Ctrl, Alt and Win shortcuts (like "Ctrl + Shift + S") near the bottom. It never shows what you type.
- A "Tutorial" template, also one click away in the Tutorial mode card: your screen with padding, rounded corners and a shadow on a dark gradient.
- `chisme record --tutorial` turns Tutorial mode on from the command line.

## [0.6.0] - 2026-10-26

### Added
- Live chat: a Chat card on the Studio page shows Twitch, Kick and YouTube messages in one list while you're live (set the channels with its gear; YouTube uses your own free API key, kept in Windows Credential Manager). Click a message to show it on screen as a branded lower third, in the recording and the streams; click again to remove it.
- Go live from the Studio page: one button starts every platform you turned on, using the keys you saved on the Live page. Clicking it again asks, then ends them all.
- Record while live (on by default; Live page, or the arrow next to Go live): Go live also records the whole stream to your Videos folder, as 16:9, 9:16 or both like Record, and it stops when the stream ends.
- Guests: "Invite a guest" makes a link for one person (type their name if you like; it fills theirs), copies it and opens the green room. The links you made are listed with Copy, and the button says what's missing if the LiveKit project isn't set up yet.
- Layer editor, rebuilt:
  - Resize from the sides as well as the corners.
  - Turn a layer with the round handle above it (Shift: steps of 15°).
  - Crop: double-click a layer or press Crop, then drag its edges to cut the picture.
  - Opacity.
  - Quick places: full frame, halves, center and corners.
  - Right-click a layer on the preview for front / back, duplicate, crop, fill / fit, straighten, place, hide and delete.
  - Arrow keys nudge (Shift: 10 px), Delete removes, Ctrl+D duplicates, Ctrl+Z / Ctrl+Y undo and redo.
  - Hovering a layer outlines it.
  - The 9:16 layout's layers have the same settings (pick 16:9 or 9:16 at the top of the Layers card, or click that preview).
  - A selected camera layer shows the camera itself: which camera, its background, and zoom, focus and light.
- Locks for Custom layouts, one for 16:9 and one for 9:16 (next to View).
- "Save layout" at the top of the Layers card saves the Custom layout (16:9 and 9:16) as a template in the studio you're in.
- Studio page Media Board: it shows only what you pin (the pin on each Media Board tile), in your order. Its Edit button lets you drag them into place, take them off and pick a color for each sound. Sounds are colored pads, lit while they play.
- Loop for Media Board clips and sounds: the ↻ button on a tile makes it start again when it ends, until you stop it.
- "Be right back" and "Starting soon" screens, in the Gaming / Stream studio and on the Templates page.
- Shortcuts for studios: Alt+Shift+1…9 switches to studio 1…9 (change it in Settings → Shortcuts). The studio picker shows each one's shortcut.
- Drag a scene in the bar under the preview to change its place.
- Templates page: "+ Add" on each template card puts it in the studio you have selected ("✓ In studio" takes it out).

### Changed
- Live page: each platform has a "Go live with Studio" toggle instead of its own Go live button, and its key is saved as you type it. While live, a platform shows its state and an End button for just that one.
- Studio page: the Media Board sits under the preview, and the right column's cards end with it (Camera, or Layers while editing, takes the height left), so there's no empty space.
- While you edit a layout, the Layers card sits beside the preview at full height (Live audio goes under it), so the preview and all the settings are visible together.
- Every scene keeps its own 9:16 layout. Changing the 9:16 layout of one scene no longer changes another, and the 9:16 layout no longer changes when you edit the 16:9 one.
- Choosing a studio on the Templates page shows only its templates; "See all" shows every one.
- A "View: 16:9 | 9:16 | Both" switch under the preview replaces the small 16:9 and 9:16 buttons on the pictures.
- When the window is narrow, the scenes move to their own row under Record instead of disappearing.
- Guests: the LiveKit settings fold behind the gear once they're set, and a project URL typed as https://… or without a prefix works too.

### Fixed
- Looping clips and sounds play again with no gap: the next pass is ready before the current one ends, so the picture never goes black between them.
- The Layers, Camera and Chat cards scroll inside the card (with a visible scroll bar) when the window is small, instead of cutting off their settings, and the Layers card no longer jumps back to the top when the Media Board has pinned items.
- The record picker stays inside a narrow preview card.

## [0.5.0] - 2026-10-25

### Added
- Studios: named sets of scenes for each kind of show. Pick one next to the scenes under the preview, and create, rename and edit them on the Templates page. Chisme starts with your current scenes plus Podcast, Content / Reels, Tutorial and Gaming / Stream.
- Vertical video. Record 16:9, 9:16 or both at once (two files, "name.mp4" and "name vertical.mp4"), from the picker next to Record. A 9:16 preview sits next to the 16:9 one; show or hide either with its 9:16 or 16:9 button (to work only in vertical, hide the 16:9 one).
- Each scene gets an automatic 9:16 layout (screen on top, people below). With Custom on, move and resize layers on the 9:16 preview too; templates keep their own 9:16 layout, and "Back to automatic" resets it.
- Streams with Vertical 9:16 on (TikTok, Instagram) send that 9:16 layout instead of a crop of the 16:9 picture.
- Scene transitions: Cut, Fade or Slide, with an adjustable length (Settings → Video). They play when you switch scene or template, in the recording, the streams and both previews.
- Command line: `--aspect horizontal|vertical|both`.

### Changed
- Studio page: Record and its buttons sit on the left under the preview, the scenes on the right and Custom on its own in the corner. Live audio and Camera fold to their title (Chisme remembers how you left them).
- The "Show Chisme in recording" tip says it only matters while recording or streaming.

### Removed
- Import and export of .chisme template files.

## [0.4.4] - 2026-10-25

### Fixed
- Clicks land on the button under the pointer. On some PCs, hiding Chisme from screen capture drew the window offset from where clicks land, so a nearby button reacted instead. Chisme now hides itself only while recording or streaming.
- The Media volume also sets what you hear. With system audio on, Media Board sounds reach the recording through your speakers, so the slider did nothing there; now it changes the sound itself, and what you hear is what goes out.

## [0.4.3] - 2026-10-24

### Added
- A Media volume slider and mute in Live audio, next to System and Microphone. It sets how loud Media Board sounds and clips are in the recording and the streams.

### Changed
- A much better virtual background. Edges follow your hair and shoulders, the old background no longer shows around you, the blur is smooth (no blocks) and has no halo, a picture background blends with your edges like a real room, and stray patches the model mistakes for you (a lamp, a piece of wall) are dropped.
- Updated eframe to 0.37, global-hotkey to 0.9 and tract-onnx to 0.23.9.

### Fixed
- Camera zoom, pan and tilt: each click keeps going the same way and the sliders follow it. Before, a slow motor or a camera that reports pan reversed (OBSBOT) made the next click go back or stall.
- With a template on, the camera's shape, size, border, shadow, mirror and corner change the template's camera. Before, they did nothing.
- The Custom button works with a template on: the template becomes the starting point of your own layout.
- Subtitles read the whole audio track. Some videos (AAC at 22 kHz) lost most of their audio and came out empty or wrong.
- The command line shows "Transcribing" in English, and `chisme marker` names the right chapters file.

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

[Unreleased]: https://github.com/LordVayron/chisme/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/LordVayron/chisme/releases/tag/v0.7.0
[0.6.0]: https://github.com/LordVayron/chisme/releases/tag/v0.6.0
[0.5.0]: https://github.com/LordVayron/chisme/releases/tag/v0.5.0
[0.4.4]: https://github.com/LordVayron/chisme/releases/tag/v0.4.4
[0.4.3]: https://github.com/LordVayron/chisme/releases/tag/v0.4.3
[0.4.2]: https://github.com/LordVayron/chisme/releases/tag/v0.4.2
[0.4.1]: https://github.com/LordVayron/chisme/releases/tag/v0.4.1
[0.4.0]: https://github.com/LordVayron/chisme/releases/tag/v0.4.0
[0.3.1]: https://github.com/LordVayron/chisme/releases/tag/v0.3.1
[0.3.0]: https://github.com/LordVayron/chisme/releases/tag/v0.3.0
[0.2.1]: https://github.com/LordVayron/chisme/releases/tag/v0.2.1
[0.2.0]: https://github.com/LordVayron/chisme/releases/tag/v0.2.0
