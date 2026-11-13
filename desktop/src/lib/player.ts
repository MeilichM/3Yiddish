import { invoke } from "@tauri-apps/api/core";
import { api } from "@/lib/api";

/** What the voice notes use of an audio element, so Linux can play on the speakers instead. */
export interface Playable {
  readonly paused: boolean;
  readonly duration: number;
  currentTime: number;
  ontimeupdate: ((ev: Event) => unknown) | null;
  onplay: ((ev: Event) => unknown) | null;
  onpause: ((ev: Event) => unknown) | null;
  onended: ((ev: Event) => unknown) | null;
  play(): Promise<void>;
  pause(): void;
}

/**
 * On Linux the window's own audio goes through GStreamer, and without its audio plugins
 * (gst-plugins-good) pressing play takes the window down. There the app plays the file itself
 * (`play_*` in lib.rs), as the terminal app does. Mac and Windows play in the window.
 */
const native = navigator.userAgent.includes("Linux") && !navigator.userAgent.includes("Android");

/** A player for the recording at `path`; `url` gives the window's URL for it when needed. */
export async function playable(path: string, url: () => Promise<string> = () => api.audioUrl(path)): Promise<Playable> {
  return native ? new Speaker(path) : new Audio(await url());
}

/** Plays on the speakers through the app; one at a time, like the window's own audio here. */
class Speaker implements Playable {
  paused = true;
  duration = NaN;
  ontimeupdate: ((ev: Event) => unknown) | null = null;
  onplay: ((ev: Event) => unknown) | null = null;
  onpause: ((ev: Event) => unknown) | null = null;
  onended: ((ev: Event) => unknown) | null = null;
  private at = 0;
  /** The app's number for this playback while it holds the speakers. */
  private id: number | null = null;
  private timer: number | undefined;

  constructor(private path: string) {}

  get currentTime() {
    return this.at;
  }

  set currentTime(at: number) {
    this.at = Math.max(0, at);
    if (this.id !== null) void invoke("play_seek", { id: this.id, at: this.at });
  }

  async play() {
    if (!this.paused) return;
    if (this.id !== null && !(await invoke<boolean>("play_pause", { id: this.id, paused: false }))) this.id = null;
    if (this.id === null) {
      const [id, seconds] = await invoke<[number, number | null]>("play_start", { path: this.path, at: this.at });
      this.id = id;
      if (seconds) this.duration = seconds;
    }
    this.paused = false;
    this.onplay?.(new Event("play"));
    this.timer = window.setInterval(() => void this.tick(), 100);
  }

  pause() {
    if (this.paused) return;
    if (this.id !== null) void invoke("play_pause", { id: this.id, paused: true });
    this.halt();
    this.onpause?.(new Event("pause"));
  }

  private halt() {
    this.paused = true;
    window.clearInterval(this.timer);
  }

  private async tick() {
    if (this.id === null || this.paused) return;
    const now = await invoke<{ at: number; done: boolean } | null>("play_position", { id: this.id });
    if (this.paused) return;
    if (!now) {
      // Another recording took the speakers.
      this.id = null;
      this.halt();
      this.onpause?.(new Event("pause"));
    } else if (now.done) {
      this.id = null;
      this.at = 0;
      this.halt();
      this.onpause?.(new Event("pause"));
      this.onended?.(new Event("ended"));
    } else {
      this.at = now.at;
      this.ontimeupdate?.(new Event("timeupdate"));
    }
  }
}
