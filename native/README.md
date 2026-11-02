# MLauncher native overlay (experimental)

Optional x64 Windows DLL for **our own launched Java child only**. This is an OpenGL renderer/input hook, not a Minecraft mod and not another topmost window. Injection is explicit opt-in and disabled by default. Existing `overlay` (companion-window) settings do not enable native injection; the new `native_overlay` setting requires fresh consent.

- **Shift+Tab** opens/closes the overlay; Esc closes it; clicking outside the overlay closes it. Hotkey repeat does not toggle it repeatedly. Input is polled via `GetAsyncKeyState` as well as the window procedure, so it works when a loader/window wrapper does not forward the key message.
- Resources: Modrinth/CurseForge search, sorting, installed marker, compatible download into this session's instance. CurseForge still needs the user's API key in the launcher.
- Logs: bounded, redacted tail of this session's log. No remote reporting.
- Closing releases input to Minecraft. Alt+Tab hides it. No keyboard/process hooks are installed globally.
- The overlay uses the launcher's accent, light/dark underlay and simple effect (waves/bubbles/aurora). It is not an always-on-top window.
- Windows x64, WGL, OpenGL 3+. Window class names are not required, so wrappers used by NeoForge/GLFW are accepted. **Vulkan / VulkanMod, EGL/ANGLE and DirectX are not supported.** Context recreation mods and other overlays can conflict. No claim of all-loader/game-version compatibility.
- Anticheats and security software may block or flag DLL injection. We do not bypass protections. Disable the setting and restart the client if it conflicts.

The launcher binds a random port on `127.0.0.1` with a separate random per-session token. That token is passed through the child environment, not command-line arguments. The bridge rejects browser Origin headers, requires the token and exact loopback Host, limits headers/bodies and concurrent connections, and never accepts arbitrary download URLs or target paths. Download IDs must first come from its own resource search. The selected instance is fixed for the lifetime of the bridge. Hiding the overlay stops polling; downloads already requested may finish. The launcher must remain open.

The launcher verifies SHA256 of the adjacent DLL against the digest embedded at build time. Missing/mismatched DLLs disable the overlay without blocking the game. Initialization/renderer messages appear in the launcher log. Changing the checkbox affects **future launches**; no live unhook/unload is attempted.

## Build

`powershell -ExecutionPolicy Bypass -File scripts/build-overlay.ps1 -TestHost`

Requires x64 MinGW GCC/G++. Dependencies are pinned by SHA256 in that script:

- Dear ImGui v1.91.9b (MIT), including its Win32/OpenGL3 backends.
- MinHook v1.3.4 (BSD-style, see bundled license including HDE notices).
- nlohmann/json v3.11.3 (MIT).

Release packaging builds the DLL, embeds its digest into the launcher, and includes the DLL and third-party licenses. Lite does not build, package or inject the DLL. Dependency archives/expanded sources/generated binaries are excluded from git.

## Isolated native smoke test

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build-overlay.ps1 -TestHost
$env:MLAUNCHER_OVERLAY_SHA256 = (Get-FileHash native/bin/mlauncher-overlay.dll -Algorithm SHA256).Hash
$env:MLAUNCHER_OVERLAY_SCREENSHOT = "$env:LOCALAPPDATA\Temp\opencode\native-overlay-smoke.bmp"
cargo run -- --overlay-smoke-test
```

This creates **a new test OpenGL process**, injects the real DLL, checks native rendering initialization and Shift+Tab closing/reopening/repeat handling, and saves the framebuffer. It never attaches to an existing game or edits the user's launcher configuration. Passing it is **not** a full Minecraft/anticheat/fullscreen compatibility test.

For the optional network-dependent catalog/download check, also set `$env:MLAUNCHER_OVERLAY_TEST_QUERY = 'Default Dark Mode'` before running. The native WinHTTP client will search real Modrinth cards; the Rust test then downloads a compatible resourcepack twice into its temporary instance and checks the registry. No user game files are touched. The test also checks that open-overlay keyboard/mouse messages do not reach the host, and that closing restores them.
