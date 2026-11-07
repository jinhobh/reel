# reel

A minimal WebKitGTK browser for watching media on Linux, built with wry and the
`adblock` crate. One window, one view, no toolbar: the page gets the whole
window, and the address bar slides in only when asked for.

## Build

```sh
sudo pacman -S --needed webkit2gtk-4.1 gst-plugins-base gst-plugins-good \
    gst-plugins-bad gst-libav gst-plugin-va
cargo build --release        # target/release/reel, ~2.3 MB
```

`gst-plugin-va` provides VA-API hardware decoding (AMD/Intel via Mesa or
intel-media-driver). Without it, video decodes on the CPU.

To install for your user and add **Reel** to the app launcher:

```sh
sudo pacman -S --needed make desktop-file-utils
make install
```

This builds the release binary and installs it in `~/.local/bin`, with a desktop
entry and icon under `${XDG_DATA_HOME:-~/.local/share}`. The launcher uses the
binary's absolute path, so it works even if `~/.local/bin` is not on the desktop
session's `PATH`. Run `make install` again after updating the code.

## Use

```
reel [--private] [--no-adblock] [--update-filters] [--hw-codecs] [URL | FILE | search terms]
```

| Key | Action |
| --- | --- |
| Ctrl+L, F6 | address bar (Esc closes) |
| Alt+Left / Alt+Right, mouse back/forward | history |
| F5, Ctrl+R (Shift: bypass cache) | reload |
| F11 | fullscreen |
| Ctrl+M | mute |
| Ctrl+= / Ctrl+- / Ctrl+0 | zoom |
| Ctrl+Shift+P | keep window on top |
| Ctrl+Shift+M | pause the page and open it in mpv |
| Ctrl+Q, Ctrl+W | quit |

- `--private`: WebKit ephemeral session; cookies and storage stay in memory.
- `--hw-codecs`: report VP9/AV1 as unsupported unless a hardware decoder exists,
  so YouTube serves a codec the GPU can decode.
- `REEL_SEARCH`: search URL prefix (default DuckDuckGo).
- Local files work: `reel ~/Videos/clip.mp4`.

## How blocking works

EasyList, EasyPrivacy and uBlock's filter and privacy lists are converted by the
`adblock` crate into WebKit content-blocker rules (~120k). WebKit compiles them
once into a native matcher that runs inside its own processes, so there is no
per-request callback into Rust. The compiled filter is cached in
`~/.cache/reel/filters` and refreshed every 4 days (`--update-filters` forces
it). Extra rules can go in `~/.config/reel/filters.txt`.

Generic cosmetic rules are dropped on purpose: they would add ~14k CSS
selectors to every page. Site-specific hiding is kept.

YouTube serves ads from the same hosts as the video, so `src/youtube.js` strips
ad data from player responses and skips any ad that still appears. This is
best effort and may break when YouTube changes. Twitch ads are spliced into
the stream and are not blocked.

## Defaults

- Web processes run in the bubblewrap sandbox (`WEBKIT_FORCE_SANDBOX`).
- Intelligent Tracking Prevention is on.
- Camera/microphone, WebRTC, and DRM (EME) are off; every permission prompt is
  denied. DRM-only services (Netflix, Spotify web) will not play.
- Script-opened popups are dropped; user-initiated new windows open in place.
- The window title always shows the page's host.
- Data: `~/.local/share/reel`.

## Measured (Radeon 860M, Wayland, release build)

- Launch to loaded local page with filters cached: ~210–260 ms.
- First run (download, convert, compile filters): ~5 s.
- Idle memory, all processes: ~183 MB PSS.
- Blocking test: GTM, Google Analytics, DoubleClick, AdSense, and Meta Pixel
  scripts blocked; a cdnjs control loaded.
