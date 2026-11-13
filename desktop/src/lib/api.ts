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

/** One marked span of a take's draft (word numbers; start == end: a missing word there). */
export interface Mark {
  start: number;
  end: number;
  heard?: string;
  /** What it should be; null: no correction, only flagged. */
  right: string | null;
  reason: string;
  note: string;
}

export type TakeStatus = "pending" | "correct" | "fixed" | "tossed";

/** A Contribute take, as kept by the core's free bench. */
export interface Take {
  n: number;
  seconds: number;
  draft: string;
  words: string[];
  final: string;
  typed: boolean;
  status: TakeStatus;
  marks: Mark[];
  sent?: string | null;
}

export interface ContributeState {
  /** The Contribute explanation was accepted here. */
  accepted: boolean;
  owner: boolean;
  /** How many takes make the set; 0 for a contributor. */
  goal: number;
  canSend: boolean;
  items: Take[];
  idea: string;
  name: string;
  sharing: boolean;
}

export interface Review {
  marks: Mark[];
  typed: string | null;
  toss: boolean;
}

export const api = {
  contributeState: () => invoke<ContributeState>("contribute_state"),
  contributeSet: (on: boolean) => invoke<{ accepted: boolean; sharing: boolean }>("contribute_set", { on }),
  ideaNext: () => invoke<string>("idea_next"),
  takeFinish: () => invoke<Take>("take_finish"),
  takeFrom: (path: string, text: string, engine?: string) => invoke<Take>("take_from", { path, text, engine }),
  takeRetry: () => invoke<Take>("take_retry"),
  takeReview: (n: number, review: Review) => invoke<Take>("take_review", { n, review }),
  takeDelete: (n: number) => invoke<void>("take_delete", { n }),
  takeClip: (n: number) => invoke<Clip>("take_clip", { n }),
  sendTakes: (name: string) => invoke<{ sent: number; left: number }>("send_takes", { name }),

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
  transcribe: (path: string, recorded: boolean) => invoke<Transcript>("transcribe", { path, recorded }),
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
