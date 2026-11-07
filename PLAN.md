# Private media browser: initial engineering plan

Status: planning draft, 2026-11-06. Project name is a placeholder.

## Product decisions

Confirmed by the user:

- Linux desktop is the first platform.
- YouTube, Twitch, and ordinary web video are the first media targets.
- The browser should prioritize security, privacy, low resource use, and fast playback.

Pending: the user's choice between fast private browsing, stronger fingerprint resistance with compatibility costs, and network anonymity. Until answered, this draft assumes strong tracker protection and private sessions with fast playback. That assumption is reversible and is not a claim of anonymity.

Initial scope assumptions: Linux x86-64; one reference distribution and laptop for the first supported configuration; Wayland preferred, with X11 compatibility evaluated. Hardware and distribution remain to be selected. No implementation or engine downloads are part of this planning task.

The intended product is a focused browser with a compact toolbar, tabs, and a viewing mode. The first release should make opening a video, watching it smoothly, and ending its session straightforward. It must keep the website origin visible outside fullscreen and identify fullscreen entry to reduce spoofing.

Security and privacy are release requirements. Performance work must preserve them. A small shell will not make a modern web engine a tiny application; measure the whole application, including shared libraries, renderers, GPU resources, and updates.

## Threat model and claims

Design against malicious pages and advertisements, compromised renderer processes, cross-site tracking, unexpected browser background traffic, unauthorized device access, and routine local browsing traces.

The browser can reduce tracking and contain web content. It cannot make a signed-in YouTube account anonymous to Google, hide a direct connection's IP address from a destination, or protect browsing on an already compromised operating system. Private session storage also cannot guarantee forensic erasure from swap, crash dumps, or external system logs.

Treat fingerprint resistance as a separate engineering problem. A small user population, unusual API behavior, or inconsistent spoofed values may make the browser distinctive. Reuse coherent upstream protections where possible and evaluate them; do not claim Tor-equivalent anonymity or invent per-request fingerprint randomization.

Relevant foundations: [Chromium Site Isolation](https://www.chromium.org/Home/chromium-security/site-isolation/), [W3C fingerprinting guidance](https://www.w3.org/TR/fingerprinting-guidance/), and [Tor fingerprinting protections](https://support.torproject.org/tor-browser/features/fingerprinting-protections/).

## Provisional engine decision

Start with a feasibility prototype using a current Chromium Embedded Framework (CEF) release and a small native shell. Use the official C++ integration, CMake/Ninja, and windowed accelerated rendering initially. CEF Views is the first UI candidate, subject to testing on the selected Linux display stack. Keep application-owned C++ small and use strict ownership and compiler hardening. Consider Rust for substantial new parsers or policy components only when the boundary and maintained integration justify it.

This is a candidate architecture, not an assertion that CEF provides all of Chrome's browser security features. CEF supplies an embedding API and Chromium-based runtime; the host still owns important policies, permission UI, packaging, and lifecycle decisions. See [CEF general usage](https://github.com/chromiumembedded/cef/blob/master/docs/general_usage.md).

| Candidate | Why consider it | Decision condition |
| --- | --- | --- |
| Current CEF with a native shell | Small application layer and Chromium media integration | First candidate; must pass security, codec, update, and GPU gates |
| Chromium with a small downstream patch set | Retain more of the complete browser's existing behavior | Prefer if CEF requires extensive security reimplementation or delays patches |
| WebKitGTK with a native shell | Linux embedding APIs and a different dependency footprint | Evaluate if Chromium cannot meet resource goals; independently verify security and playback |
| Firefox downstream | Existing privacy and browser features | Reconsider if maximum fingerprint resistance becomes the main requirement |

These are engineering judgments to validate, not measured rankings of speed, memory, or security. Do not build a new HTML engine, JavaScript runtime, codec stack, or TLS implementation. Keep one engine in the shipped application.

The engine decision must pass four gates before substantial UI work:

1. The packaged application preserves working renderer sandboxing and site isolation, including cross-site frames. Confirm the actual process and OS restrictions; command-line settings alone are insufficient evidence. Reject configurations that need sandbox-disabling flags.
2. YouTube and Twitch play using the required codecs. Test H.264/AAC early, alongside VP9/Opus and AV1 where available. Stock engine binaries may not contain every codec needed by the target services.
3. Hardware decoding works on the selected reference GPU without weakening the GPU sandbox or using unsupported security workarounds. Establish separate support results for Intel, AMD, and NVIDIA before advertising them.
4. The project can continuously obtain, build, validate, and distribute current engine security fixes. Measure total delay from upstream publication, including embedding-framework and packaging delay.

Chromium's documented [codec matrix](https://www.chromium.org/audio-video/) distinguishes H.264/AAC from its open codecs. A build flag alone does not establish redistribution rights. Confirm the distribution route before committing to a codec build. DRM is outside the first version; a later Widevine integration would require a separate compatibility and distribution decision, informed by [Widevine access requirements](https://developers.google.com/widevine/access).

## Security and privacy design

### Process and privilege boundaries

- Preserve the engine's sandbox, site isolation, same-origin checks, certificate validation, mixed-content protections, and exploit mitigations.
- Use engine processes for rendering, networking, GPU work, and media decoding as supported by the selected runtime. Document which components are actually sandboxed; do not assume one process per tab proves isolation.
- Keep navigation, permissions, and privileged UI in the browser host. Do not expose a general native bridge to websites. Validate any renderer-to-host messages against the sender, origin, allowed operation, and current navigation.
- Disable remote debugging in production. Never pass untrusted URLs or filenames to a shell. Require a user action for downloads, external protocols, or opening files with other applications.
- Default microphone, camera, location, notifications, USB, Bluetooth, serial, and screen capture to denied or explicit user grants as appropriate. Playback should not grant capture access.
- Audit which phishing and download protections the embedded runtime actually supplies. Choose a privacy-preserving approach for missing protections rather than silently assuming Chrome parity.

### Session model

- Default to an ephemeral browsing session with no saved history, no restored browsing session, and no persistent website login state.
- Offer an explicit persistent container for a service when the user wants to stay signed in. Store any future secrets through the operating system's credential facilities.
- Isolate persistent containers and private contexts. Partition third-party state by top-level site and block cross-site tracking cookies while preserving necessary same-site access.
- Reuse the engine's storage and network partitioning; a cookie allowlist alone is insufficient. Test caches, service workers, IndexedDB, storage-access grants, popups, and authentication redirects.
- A “Forget session” action must close its tabs and background workers, destroy its context, and clear its associated state. Audit normal exits, crashes, and recovery.
- Prefer memory-backed website state for private contexts where the engine supports it. Inventory remaining disk writes, including GPU caches, logs, and crash files. Describe the limits accurately.

### Network and tracking policies

- Include a maintained request-blocking engine and curated filter lists; validate rule compatibility, licensing, update authenticity, and playback breakage. Do not implement a complex filter grammar from scratch.
- Protect against tracking requests, popunders, and unsolicited navigation while preserving user-initiated actions. Do not promise universal blocking of ads embedded in a video stream.
- Provide narrowly scoped site exceptions with a visible reset action. An exception must not turn off TLS validation or sandboxing.
- Default to HTTPS, with an explicit, scoped choice when an HTTP-only site is needed. Keep certificate failures blocking in the first version.
- Disable application telemetry and automatic crash uploads. Audit actual background connections instead of relying on privacy flags.
- Necessary update and filter requests should contain no browsing history or unique installation identifier. Keep a documented endpoint inventory and distinguish browser traffic from page traffic in tests.
- Disable speculative cross-site connections and network search suggestions by default. Select a DNS policy explicitly rather than silently switching to a third-party resolver.
- Restrict WebRTC networking or disable it if unnecessary for the supported watch-only sites; verify playback compatibility. Capture permissions and connection/IP exposure are different controls.
- Defer any anonymity-network mode to a separately tested design. Do not add an unverified proxy toggle and call the result anonymous.

## Media and lightweight behavior

First-release features:

- Tabs, URL/search entry, back/forward, reload, clear origin display, and a minimal permissions/privacy panel.
- Smooth YouTube and Twitch playback, including live streams, seeking, captions, fullscreen, and picture-in-picture where supported.
- Media keys, playback speed, keyboard navigation, accessible controls, and a distraction-reduced viewing mode.
- Media-aware tab lifecycle: suspend eligible background tabs, but preserve audible playback, active picture-in-picture, and state needed to resume.
- Bounded buffers and cache usage, with explicit limits on private-session disk caching. Test seek and live-stream behavior under those limits.
- A local diagnostics view for codec, decoder, resolution, dropped frames, and engine version, with no automatic upload.

Use the engine's normal media pipeline first. Prefer hardware decoding when supported; handle a software fallback honestly and avoid claiming 4K efficiency on unsupported hardware. Keep the GPU sandbox enabled.

Defer account sync, extension marketplaces, AI sidebars, bundled VPNs, DRM services, casting, media downloading, and a separate mpv/FFmpeg playback path. Extra playback paths add dependencies and separate security/update obligations. Reassess specific features after the core browser is measured.

## Measurements and acceptance criteria

Select the reference laptop, GPU, distribution, display server, screen brightness, and power mode before setting final budgets. Compare with unmodified Chromium of a similar engine revision and a current mainstream Firefox. Use fresh profiles; separate the benefit of tracker blocking from the benefit of the shell.

| Area | Measurement | Initial target or gate |
| --- | --- | --- |
| Startup | Cold/warm usable-window time; median and p95 over repeated runs | No regression versus the equivalent engine baseline; set an absolute target after the spike |
| Memory | Total process-tree PSS, plus separately reported GPU allocation | Provisional goal: 15% lower idle/background memory than Chromium in the agreed workload; revise from data without weakening isolation |
| Playback | First frame, seeking, stalls, A/V sync, captions | Target sites and controls work consistently |
| Frame delivery | Dropped frames during a 30-minute controlled 1080p60 test after warmup | Provisional target below 0.1% on supported reference hardware |
| Efficiency | CPU, GPU/video engine use, and energy for the same codec, resolution, and frame rate | No regression versus the same-engine baseline; report software fallback separately |
| Background work | CPU wakeups and outbound requests with a blank page and suspended tabs | No unexplained recurring work or undocumented endpoints |
| Footprint | Compressed download, installed size, runtime dependencies | Report the complete footprint, including system-provided engine dependencies |
| Private state | Clean exit, forced crash, relaunch, and context-reset audit | No recoverable browsing state through normal application/profile interfaces after reset; document residual OS-level traces |
| Boundaries | Cross-site frames, renderer privileges, malicious popups, permission requests, IPC | Required protections remain active in the distributed package |
| Updates | Upstream fix to installed signed release | Provisional critical-fix target: within 48 hours of public upstream availability; prove a feasible supply path |

Performance numbers above are proposed acceptance criteria, not achieved results. Use controlled local test media to reproduce regressions and real services to test integration. Test 4K where hardware supports it; do not hide codec or quality differences in comparisons.

## Delivery and ongoing maintenance

1. Feasibility: create the minimal CEF window; verify sandbox/site isolation, H.264/AAC, YouTube/Twitch, display-stack support, GPU decoding, and resource baselines. Deliver a scorecard and a recorded engine decision. Stop or change engines when a mandatory gate fails.
2. Security foundation: implement explicit permission handling, ephemeral and persistent contexts, state partitioning verification, navigation policy, safe host IPC, and network inventory. Deliver repeatable boundary and privacy checks.
3. Media alpha: add tabs, viewing mode, captions/fullscreen/PiP integration, media keys, and media-aware suspension. Deliver a working playback and accessibility matrix.
4. Packaging and updates: pick a Linux packaging route after testing nested sandbox compatibility. Prefer distribution-managed signed updates where practical; otherwise use a well-reviewed signed updater with expiry and anti-rollback metadata. Test valid updates, tampered packages, stale metadata, interrupted updates, and recovery without silent security downgrades.
5. Private beta: validate on the declared GPU/distribution matrix; run targeted review and fuzzing of new parsers/IPC; audit background traffic and private-state behavior; publish supported configurations and measured limitations.

Keep a small downstream patch set, pinned dependency inputs, license notices, an SBOM, artifact provenance, and a reproducible build goal. The signing process needs protected keys, restricted CI access, and a documented recovery path. Maintain current engine and filter updates independently of feature development.

The update target includes delays before an embedded engine release is available. If that delay cannot meet the requirement, use a maintained self-build path or revisit the engine choice. Private release readiness requires that one full engine update and one tamper-rejection exercise have already succeeded.

## Next decisions

The privacy preference is the one currently pending user question. Reference GPU/distribution and personal-use versus public distribution can be selected before implementation; the latter affects codec distribution and release infrastructure. These decisions do not prevent this initial plan from being reviewed.

The next implementation milestone, when requested, is the feasibility prototype and scorecard. Visual polish follows evidence that the browser can be secure, compatible, maintainable, and efficient.
