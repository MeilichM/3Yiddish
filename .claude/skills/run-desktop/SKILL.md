---
name: run-desktop
description: Launch the 3Yiddish desktop app (Tauri, Windows) against a throwaway test home and drive it - click the header, switch models, CPU/GPU, paste a recording and read the transcription, take screenshots. Use to run, test or screenshot the desktop app.
---

# Run and drive the desktop app (Windows)

Verified on Windows 11 with Visual Studio 2022 Community. Scripts are in `scripts/` next to this
file; paths below are relative to the repo root.

## Once per machine

- Visual Studio 2022 with **Desktop development with C++**, plus the components
  `VC.Tools.x86.x64`, `Windows11SDK.26100`, `VC.CMake.Project` and `VC.Llvm.Clang`
  (whisper.cpp needs CMake; bindgen needs libclang). Without them linking fails with
  `LNK1104: cannot open file 'msvcrt.lib'`. Install from the command line with the
  Visual Studio Installer's `setup.exe modify --installPath <VS> --add <id> ... --passive --norestart`
  run as admin (`Start-Process -Verb RunAs`). Do **not** pass `--wait`: `setup.exe` rejects it
  (exit 87). The person has to click Yes on the admin prompt.
- `rustup component add rustfmt clippy`
- The driver: `cd .claude/skills/run-desktop/scripts && npm install` (from inside that folder;
  `npm install --prefix` on Windows looks for a package.json in the current folder instead)
- For GPU runs only: the Vulkan SDK, with `VULKAN_SDK` set.

## 1. Make a test home (never use the real one)

```powershell
powershell -File .claude/skills/run-desktop/scripts/setup-testhome.ps1
```

Creates `%TEMP%\3y-desktop-test`: the installed model hard-linked in, a second hand-placed copy
labelled "Hand copy" (so the model menu lists two models), and `audio\sample.wav` taken from the
real home's recordings. The real `%APPDATA%\3yiddish` is only read. Re-running resets the models.

## 2. Launch (in the background; it keeps running)

```powershell
cmd /c .claude\skills\run-desktop\scripts\launch.cmd              # CPU only, quick
cmd /c .claude\skills\run-desktop\scripts\launch.cmd "" gpu       # with Vulkan
```

Run it with `run_in_background` and log to a file. It sets `THREE_YIDDISH_HOME` to the test home,
gives WebView2 its own profile (the real app's saved history stays untouched) and opens the
debug port 9333. Ready when `curl -s localhost:9333/json/version` answers (first build: about
2-3 minutes; a GPU build compiles whisper.cpp with Vulkan into `C:\t3y`).

- Without `gpu`, the GPU item in the CPU/GPU menu is greyed out. That is the build, not a bug.
- `gpu` must build in a short folder (`C:\t3y`): in the repo folder the Vulkan shader step fails
  with `No CMAKE_C_COMPILER could be found` / `MSB8066`, which is Windows' 260-character path
  limit (the release workflow uses `C:\t` for the same reason).

## 3. Drive it

```bash
cd .claude/skills/run-desktop/scripts
node drive.mjs shot1 "await page.waitForTimeout(1000); return await page.evaluate(() => document.body.innerText)"
```

`page` is a Playwright page on the app window and `fs` is Node's fs. Screenshots land in
`%TEMP%\3y-desktop-test\shots\`. **Look at them** (Read the PNG). Useful moves:

- Model menu: `page.getByRole('button', {name: /^Model /}).click()`, then
  `getByRole('menuitemradio', {name: /Hand copy/})` or `getByRole('menuitem', {name: /newer model/})`.
- CPU/GPU: `page.getByRole('button', {name: /^CPU|^GPU/}).click()`, then
  `getByRole('menuitemradio', {name: 'GPU'})`.
- Messages (toasts): `[...document.querySelectorAll('[data-sonner-toast]')].map(t => t.innerText)`.
  They vanish after about 4 seconds, so poll right after the click.
- Transcribe a recording (the app's real paste path, `import_bytes` then `transcribe`):

  ```js
  const b64 = fs.readFileSync(process.env.TEMP + '/3y-desktop-test/audio/sample.wav').toString('base64');
  await page.evaluate((b64) => {
    const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
    const dt = new DataTransfer(); dt.items.add(new File([bytes], 'sample.wav', {type: 'audio/wav'}));
    document.body.dispatchEvent(new ClipboardEvent('paste', {clipboardData: dt, bubbles: true, cancelable: true}));
  }, b64);
  // then poll document.body.innerText for Hebrew letters and "<n>s on CPU|GPU"
  ```

- Check the switch on disk: `state\models\current` and `previous` in the test home.
- Edits to `desktop/src` hot-reload; edits to `desktop/src-tauri` rebuild and restart the app.

Not drivable this way: the microphone (no voice), native file dialogs, and the global
push-to-talk shortcut.

## 4. Stop

The `cmd` wrapper's exit does not take the app with it. Kill the tree:

```powershell
Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object { $_.CommandLine -match 'tauri|vite' } | ForEach-Object { taskkill /PID $_.ProcessId /T /F }
Get-Process 3yiddish-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
```

`taskkill` may print `The process "<n>" not found` for processes the first tree kill already
ended, and the background task then reports "failed". Both are the kill, not a crash.
