# 3Yiddish desktop

The same 3Yiddish, in its own window instead of a terminal. Click the mic (or hold
**Space**), speak, and the Yiddish appears. Click it to copy. Drop or paste a recording to
transcribe it. Everything runs on your computer; nothing is sent anywhere.

**Push-to-talk:** hold **F9** in any program, speak, let go, and the Yiddish is typed where
your cursor is. Pick another key or combination (Ctrl+Shift+Y, Caps Lock, a mouse-keyboard
macro key...) in **Settings** (the gear), or switch it off there. While it is on, closing
the window keeps 3Yiddish in the tray so the shortcut keeps working; **Start with Windows**
in Settings starts it there at sign-in.

**Contribute** (top of the window) is the terminal app's playground: say something in your
own words, read what the model wrote, click the words it got wrong and fix them, and press
**Send** when you have a few. It shows what contributing means first and keeps the takes in
the same place as the terminal app (`state/contributions/`). **Share transcriptions** in
Settings is the terminal app's sharing: asked once, off unless you say yes.

It is a [Tauri](https://tauri.app) app: the speech engine is the 3Yiddish Rust library in this
repository (`../src`), called directly in the same process. The window is React with
[shadcn/ui](https://ui.shadcn.com) components; JavaScript is only used to build it, nothing
extra is installed on the computer it runs on.

The desktop app and the terminal app share their data folder, so a model downloaded by
either one is used by both. The desktop app keeps its own recordings and settings in
`state/desktop/` there.

## Build it

You need Rust, Node.js 20+, and what whisper.cpp needs to compile: a C++ compiler, CMake
3.21+ and libclang. On Linux also the [Tauri system packages](https://tauri.app/start/prerequisites/).

```sh
cd desktop
npm install
npm run tauri dev                         # run it while developing
npm run tauri build                       # installers (Windows: .msi and setup .exe)
npm run tauri build -- --no-bundle        # just the program
npm run tauri build -- --features vulkan  # graphics card build (Windows, Linux)
```

The graphics-card build needs the [Vulkan SDK](https://vulkan.lunarg.com/) to compile, and
only the graphics driver to run. Macs always use the Apple GPU.

### Windows

Run every build command through `scripts\win-env.ps1`:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\win-env.ps1 npx tauri build --features vulkan
```

It finds a Visual Studio with the C++ libraries, CMake, Ninja, libclang and the Vulkan SDK,
and builds into `%USERPROFILE%\.3yb` (the program and the installers end up in
`%USERPROFILE%\.3yb\release`). The short folder matters: the GPU build nests files so deep
that the compiler cannot open them under a long path like Documents\GitHub\....

Missing pieces, all per-user (no admin):

```powershell
pip install --target $env:USERPROFILE\.libclang libclang
pip install --target $env:USERPROFILE\.cmake cmake
pip install --target $env:USERPROFILE\.ninja ninja
```

## Layout

- `src-tauri/src/lib.rs`: the Rust commands the window calls (microphone, transcribe, model
  download, GPU/CPU, tray, start at sign-in).
- `src-tauri/src/ptt.rs`: push-to-talk (the global shortcut, record, transcribe, type).
- `src-tauri/src/contribute.rs`: Contribute and sharing, on the core's `freebench`, `phone`
  and `share`.
- `src/App.tsx`: the main screen. `src/components/`: voice notes, the composer, first-run
  setup, settings.
- `src/components/ui/`: shadcn components (copied in, edit freely).
- `app-icon.svg`: the icon; `npx tauri icon app-icon.svg` regenerates `src-tauri/icons/`.
