import { useEffect, useState } from "react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Waveform } from "@/components/composer";
import { duration } from "@/components/voice-note";

const BARS = 40;

type State = "idle" | "recording" | "transcribing" | { message: string };

/**
 * The push-to-talk pill, in its own small window at the bottom of the screen (src-tauri's
 * pill.rs shows and hides it): the voice's waveform while the shortcut is held, then
 * "Transcribing" until the words are typed.
 */
export function Pill() {
  const [state, setState] = useState<State>("idle");
  const [levels, setLevels] = useState<number[]>(() => Array(BARS).fill(0));
  const [seconds, setSeconds] = useState(0);
  const [fading, setFading] = useState(false);

  useEffect(() => {
    const said = (message: string) => {
      setFading(false);
      setState({ message });
    };
    const off = api.onPtt((e) => {
      switch (e.state) {
        case "recording":
          setLevels(Array(BARS).fill(0));
          setSeconds(0);
          setFading(false);
          setState("recording");
          break;
        case "transcribing":
          setState("transcribing");
          break;
        case "cancelled":
          if (e.reason) said("Didn't catch that");
          else setState("idle");
          break;
        case "done":
          if (e.transcript.text.trim()) setState("idle");
          else said("Didn't catch that");
          break;
        case "error":
          said("Couldn't transcribe that");
          break;
      }
    });
    // The theme chosen in the main window, followed here too.
    const theme = (e: StorageEvent) => {
      if (e.key === "3yiddish.theme") location.reload();
    };
    window.addEventListener("storage", theme);
    return () => {
      void off.then((f) => f());
      window.removeEventListener("storage", theme);
    };
  }, []);

  // The voice, as it goes.
  useEffect(() => {
    if (state !== "recording") return;
    const timer = setInterval(async () => {
      const l = await api.recordLevel();
      setSeconds(l.seconds);
      setLevels((prev) => [...prev.slice(1), l.level]);
    }, 60);
    return () => clearInterval(timer);
  }, [state]);

  // A message fades out just before the window hides.
  useEffect(() => {
    if (typeof state !== "object") return;
    const fade = setTimeout(() => setFading(true), 1200);
    const clear = setTimeout(() => setState("idle"), 1700);
    return () => {
      clearTimeout(fade);
      clearTimeout(clear);
    };
  }, [state]);

  if (state === "idle") return null;
  return (
    <div className="flex h-full items-center justify-center">
      <div
        className={cn(
          "flex h-10 w-[200px] items-center gap-2.5 rounded-full border bg-card px-4 text-sm text-muted-foreground shadow-lg shadow-black/15 transition-opacity duration-300",
          fading && "opacity-0",
        )}
      >
        {state === "recording" ? (
          <>
            <span className="size-2 shrink-0 animate-pulse rounded-full bg-red-500" />
            <Waveform levels={levels} />
            <span className="shrink-0 text-xs tabular-nums text-foreground">{duration(seconds)}</span>
          </>
        ) : state === "transcribing" ? (
          <span className="flex w-full items-center justify-center gap-2.5">
            <span>Transcribing</span>
            <span className="flex gap-1">
              {[0, 1, 2].map((i) => (
                <span
                  key={i}
                  className="size-1.5 animate-bounce rounded-full bg-muted-foreground"
                  style={{ animationDelay: `${i * 150}ms` }}
                />
              ))}
            </span>
          </span>
        ) : (
          <span className="w-full truncate text-center">{state.message}</span>
        )}
      </div>
    </div>
  );
}
