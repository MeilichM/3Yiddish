import { useRef } from "react";
import { ArrowUp, Mic, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { duration } from "@/components/voice-note";
import { cn } from "@/lib/utils";

const ACCEPT = "audio/*,video/mp4,.wav,.mp3,.m4a,.aac,.ogg,.oga,.opus,.flac,.mp4";

export function Composer({
  recording,
  pushToTalk,
  levels,
  seconds,
  disabled,
  onStart,
  onStop,
  onCancel,
  onFiles,
}: {
  recording: boolean;
  /** The take is held by the push-to-talk shortcut: letting go of it ends the take. */
  pushToTalk: boolean;
  /** Recent microphone loudness, oldest first, 0 to 1. */
  levels: number[];
  seconds: number;
  disabled: boolean;
  onStart: () => void;
  onStop: () => void;
  onCancel: () => void;
  onFiles: (files: File[]) => void;
}) {
  const picker = useRef<HTMLInputElement>(null);

  return (
    <div className="flex items-center gap-2 rounded-2xl border bg-card p-2 shadow-sm">
      <input
        ref={picker}
        type="file"
        accept={ACCEPT}
        multiple
        hidden
        onChange={(e) => {
          onFiles(Array.from(e.target.files ?? []));
          e.target.value = "";
        }}
      />
      {recording ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button size="icon" variant="ghost" className="rounded-xl" onClick={onCancel} aria-label="Cancel">
              <X />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Cancel (Esc)</TooltipContent>
        </Tooltip>
      ) : (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              size="icon"
              variant="ghost"
              className="rounded-xl"
              onClick={() => picker.current?.click()}
              disabled={disabled}
              aria-label="Add a recording"
            >
              <Plus />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Add a recording</TooltipContent>
        </Tooltip>
      )}

      <div className="flex h-9 min-w-0 flex-1 items-center gap-3 px-1 text-sm text-muted-foreground">
        {recording ? (
          <>
            <span className="size-2 shrink-0 animate-pulse rounded-full bg-red-500" />
            <span className="w-10 shrink-0 tabular-nums text-foreground">{duration(seconds)}</span>
            {pushToTalk && <span className="shrink-0 text-xs">Push-to-talk: let go to finish</span>}
            <div className="flex h-7 flex-1 items-center justify-end gap-[3px] overflow-hidden">
              {levels.map((l, i) => (
                <span
                  key={i}
                  className="w-[3px] shrink-0 rounded-full bg-foreground/70"
                  style={{ height: `${Math.max(8, Math.min(100, l * 140))}%` }}
                />
              ))}
            </div>
          </>
        ) : (
          <span className="truncate">
            {disabled ? "Waiting for the speech model…" : "Hold Space to talk, or drop / paste a recording"}
          </span>
        )}
      </div>

      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            size="icon"
            className={cn("size-9 rounded-xl", recording && "bg-red-500 text-white hover:bg-red-600")}
            onClick={recording ? onStop : onStart}
            disabled={disabled && !recording}
            aria-label={recording ? "Stop and transcribe" : "Start recording"}
          >
            {recording ? <ArrowUp /> : <Mic />}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{recording ? "Stop and transcribe" : "Record (or hold Space)"}</TooltipContent>
      </Tooltip>
    </div>
  );
}
