import { useEffect, useRef, useState } from "react";
import { Pause, Play } from "lucide-react";
import { toast } from "sonner";
import { api, message, type Clip } from "@/lib/api";
import { playable, type Playable } from "@/lib/player";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Playable URLs of voice notes, asked for once per file. */
const urls = new Map<string, Promise<string>>();
function audioUrl(path: string) {
  let url = urls.get(path);
  if (!url) {
    url = api.audioUrl(path);
    url.catch(() => urls.delete(path));
    urls.set(path, url);
  }
  return url;
}

export function duration(seconds: number) {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Only one voice note plays at a time. */
let playing: Playable | null = null;

export function VoiceNote({ clip, className }: { clip: Clip; className?: string }) {
  const audio = useRef<Playable | null>(null);
  const [isPlaying, setPlaying] = useState(false);
  const [at, setAt] = useState(0);
  const [missing, setMissing] = useState(false);

  useEffect(() => () => audio.current?.pause(), []);

  async function load() {
    if (audio.current) return audio.current;
    const el = await playable(clip.path, () => audioUrl(clip.path));
    el.ontimeupdate = () => setAt(el.currentTime);
    el.onplay = () => setPlaying(true);
    el.onpause = () => setPlaying(false);
    el.onended = () => {
      setPlaying(false);
      setAt(0);
    };
    audio.current = el;
    return el;
  }

  async function toggle() {
    try {
      const el = await load();
      if (el.paused) {
        if (playing && playing !== el) playing.pause();
        playing = el;
        await el.play();
      } else el.pause();
    } catch (e) {
      fail(e);
    }
  }

  function fail(e: unknown) {
    setPlaying(false);
    setMissing(true);
    toast.error(`Cannot play this recording: ${message(e)}`);
  }

  async function seek(e: React.MouseEvent<HTMLDivElement>) {
    const box = e.currentTarget.getBoundingClientRect();
    const share = Math.min(1, Math.max(0, (e.clientX - box.left) / box.width));
    try {
      const el = await load();
      el.currentTime = share * (el.duration || clip.seconds);
      setAt(el.currentTime);
      if (el.paused) void toggle();
    } catch (e) {
      fail(e);
    }
  }

  const done = clip.seconds > 0 ? at / clip.seconds : 0;
  return (
    <div
      className={cn(
        "flex w-full max-w-sm items-center gap-3 rounded-2xl bg-primary px-3 py-2 text-primary-foreground",
        className,
      )}
    >
      <Button
        size="icon"
        variant="secondary"
        className="size-8 shrink-0 rounded-full"
        onClick={toggle}
        disabled={missing}
        aria-label={isPlaying ? "Pause" : "Play"}
      >
        {isPlaying ? <Pause className="fill-current" /> : <Play className="fill-current" />}
      </Button>
      <div className="flex h-8 flex-1 cursor-pointer items-center gap-[2px]" onClick={seek}>
        {clip.peaks.map((p, i) => (
          <span
            key={i}
            className={cn(
              "w-full min-w-[2px] rounded-full bg-current transition-opacity",
              i / clip.peaks.length < done ? "opacity-100" : "opacity-40",
            )}
            style={{ height: `${Math.max(10, p * 100)}%` }}
          />
        ))}
      </div>
      <span className="w-10 shrink-0 text-right text-xs tabular-nums opacity-80">
        {missing ? "gone" : duration(isPlaying || at > 0 ? at : clip.seconds)}
      </span>
    </div>
  );
}
