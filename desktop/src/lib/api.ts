import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Mirrors the Rust commands in src-tauri/src/lib.rs. */
export type Compute = "gpu" | "cpu";

export interface Status {
  appVersion: string;
  model: string | null;
  /** Every model on disk that can be run: its folder (version) and the name to show. */
  models: { version: string; label: string }[];
  /** The active model runs on the processor only (the Omnilingual-based one). */
  cpuOnly: boolean;
  compute: Compute;
  gpuAvailable: boolean;
  gpuName: string | null;
}

/** A model the model tag's menu offers: on disk, or published and not downloaded yet. */
export interface Listed {
  version: string;
  label: string;
  /** Bytes a download takes (0 for one that is only on disk). */
  size: number;
  installed: boolean;
}

/** One kind of model on this computer, as Settings lists it. */
export interface Family {
  /** "Whisper" or "Omnilingual". */
  label: string;
  /** The version in use: the active one when this kind is active, else the newest. */
  version: string;
  /** The version Roll back goes to, when one is still on disk. */
  older: string | null;
  active: boolean;
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
  /** The model that wrote it (its version). */
  model: string | null;
}

export interface Level {
  level: number;
  seconds: number;
  failed: string | null;
}

/** Chat: whether it runs on this computer, and whether it is downloaded. */
export interface ChatState {
  available: boolean;
  ready: boolean;
}

/** One message of a chat conversation, as the core takes it. */
export interface ChatTurn {
  role: "user" | "assistant";
  content: string;
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
  /** A newer version of a model on this computer is out. */
  newerModel: () => invoke<boolean>("newer_model"),
  /** The first model, or the newer versions of the ones here (never one nobody asked for). */
  pullModel: () => invoke<string>("pull_model"),
  modelCatalog: () => invoke<Listed[]>("model_catalog"),
  /** Download one published model and make it the active one. */
  downloadModel: (version: string) => invoke<string>("download_model", { version }),
  /** Each kind of model here, with the version in use and the one Roll back goes to. */
  modelFamilies: () => invoke<Family[]>("model_families"),
  /** Roll one kind of model back to its older version, which becomes the active model. */
  rollbackModel: (older: string) => invoke<void>("rollback_model", { older }),
  switchModel: (version: string) => invoke<string>("switch_model", { version }),
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
  /** `model`: an installed model's version to transcribe with; left out, the active one. */
  transcribe: (path: string, recorded: boolean, model?: string) =>
    invoke<Transcript>("transcribe", { path, recorded, model: model ?? null }),
  /** A URL the window can play the file from (streamed from disk). */
  audioUrl: async (path: string) => {
    await invoke<void>("allow_audio", { path });
    return convertFileSrc(path);
  },
  forgetAudio: (path: string) => invoke<void>("forget_audio", { path }),
  onModelProgress: (f: (p: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>("model-progress", (e) => f(e.payload)),

  chatState: () => invoke<ChatState>("chat_state"),
  /** Bytes chat takes to download here; null offline. */
  chatSize: () => invoke<number | null>("chat_size"),
  chatDownload: () => invoke<void>("chat_download"),
  onChatProgress: (f: (p: { done: number; total: number }) => void): Promise<UnlistenFn> =>
    listen<{ done: number; total: number }>("chat-progress", (e) => f(e.payload)),
  /** The answer, in Yiddish, to the conversation so far (ending with the person's message). */
  chatSend: (messages: ChatTurn[]) => invoke<string>("chat_send", { messages }),
  /** A spoken chat message's words; the recording is deleted after. */
  chatHear: (path: string) => invoke<string>("chat_hear", { path }),
};

/** Tauri rejects with the Rust error string; anything else becomes readable too. */
export function message(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
