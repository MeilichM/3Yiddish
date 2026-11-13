import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Mirrors the Rust commands in src-tauri/src/lib.rs. */
export type Compute = "gpu" | "cpu";

export interface Status {
  appVersion: string;
  model: string | null;
  compute: Compute;
  gpuAvailable: boolean;
  gpuName: string | null;
}

/** A piece of audio: a recording made here, or a file brought in. */
export interface Clip {
  id: string;
  path: string;
  name: string;
  seconds: number;
  peaks: number[];
}

export interface Transcript {
  text: string;
  ms: number;
  compute: Compute;
}

export interface Level {
  level: number;
  seconds: number;
  failed: string | null;
}

export interface Progress {
  file: string;
  done: number;
  total: number;
}

export interface ShortcutInfo {
  shortcut: string | null;
  default: string;
  error: string | null;
}

/** A push-to-talk take, as the Rust side reports it. */
export type PttEvent =
  | { state: "recording" }
  | { state: "cancelled"; reason: string }
  | { state: "transcribing"; clip: Clip }
  | { state: "done"; id: string; transcript: Transcript; typed: boolean }
  | { state: "error"; id: string; error: string };

export const api = {
  autostartGet: () => invoke<boolean>("autostart_get"),
  autostartSet: (on: boolean) => invoke<void>("autostart_set", { on }),
  shortcutGet: () => invoke<ShortcutInfo>("shortcut_get"),
  shortcutSet: (shortcut: string | null) => invoke<void>("shortcut_set", { shortcut }),
  shortcutPause: (paused: boolean) => invoke<void>("shortcut_pause", { paused }),
  onPtt: (f: (e: PttEvent) => void): Promise<UnlistenFn> =>
    listen<PttEvent>("ptt", (e) => f(e.payload)),
  status: () => invoke<Status>("status"),
  newerModel: () => invoke<string | null>("newer_model"),
  pullModel: () => invoke<string>("pull_model"),
  rollbackModel: () => invoke<string>("rollback_model"),
  warm: () => invoke<void>("warm"),
  setCompute: (mode: Compute) => invoke<void>("set_compute", { mode }),
  recordStart: () => invoke<void>("record_start"),
  recordLevel: () => invoke<Level>("record_level"),
  recordCancel: () => invoke<void>("record_cancel"),
  recordStop: () => invoke<Clip>("record_stop"),
  importPath: (path: string) => invoke<Clip>("import_path", { path }),
  importFile: async (file: File) =>
    invoke<Clip>("import_bytes", new Uint8Array(await file.arrayBuffer()), {
      headers: { "x-file-name": encodeURIComponent(file.name) },
    }),
  transcribe: (path: string) => invoke<Transcript>("transcribe", { path }),
  /** A URL the window can play the file from (streamed from disk). */
  audioUrl: async (path: string) => {
    await invoke<void>("allow_audio", { path });
    return convertFileSrc(path);
  },
  forgetAudio: (path: string) => invoke<void>("forget_audio", { path }),
  onModelProgress: (f: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("model-progress", (e) => f(e.payload)),
};

/** Tauri rejects with the Rust error string; anything else becomes readable too. */
export function message(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
