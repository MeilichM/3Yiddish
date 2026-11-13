import { Check, Copy, FileAudio, RotateCcw, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import type { Clip, Compute } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { VoiceNote } from "@/components/voice-note";

export interface Entry {
  clip: Clip;
  /** A recording made in the app (vs. a file brought in). */
  recorded: boolean;
  at: number;
  state: "transcribing" | "done" | "error";
  text?: string;
  ms?: number;
  compute?: Compute;
  error?: string;
}

function when(at: number) {
  return new Date(at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

export function EntryView({
  entry,
  onRetry,
  onRemove,
}: {
  entry: Entry;
  onRetry: () => void;
  onRemove: () => void;
}) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    if (!entry.text) return;
    await navigator.clipboard.writeText(entry.text);
    setCopied(true);
    toast.success("Copied");
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <article className="group flex flex-col gap-2">
      <div className="flex items-center justify-end gap-2">
        {!entry.recorded && (
          <span className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
            <FileAudio className="size-3.5 shrink-0" />
            <span className="truncate">{entry.clip.name}</span>
          </span>
        )}
        <VoiceNote clip={entry.clip} />
      </div>

      {/* The answer side of the chat: from the left, never the whole width. */}
      <div className="relative w-fit max-w-[85%] min-w-[min(17rem,85%)] self-start rounded-2xl border bg-card text-card-foreground">
        {entry.state === "transcribing" && (
          <div className="flex items-center justify-end gap-3 px-4 py-3 text-sm text-muted-foreground">
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
          </div>
        )}

        {entry.state === "error" && (
          <div className="flex items-center justify-between gap-3 px-4 py-3 text-sm">
            <span className="text-destructive">{entry.error}</span>
            <Button size="sm" variant="outline" onClick={onRetry}>
              <RotateCcw /> Try again
            </Button>
          </div>
        )}

        {entry.state === "done" && (
          <>
            <p
              dir="rtl"
              onClick={copy}
              title="Click to copy"
              className="cursor-pointer px-4 pt-3 pb-1 text-[1.0625rem] leading-relaxed whitespace-pre-wrap select-text"
            >
              {entry.text || <span className="text-muted-foreground">(nothing heard)</span>}
            </p>
            <div className="flex items-center justify-between px-2 pb-1.5 text-xs text-muted-foreground">
              <div className="flex items-center opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button size="icon" variant="ghost" className="size-7" onClick={copy} aria-label="Copy">
                      {copied ? <Check /> : <Copy />}
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Copy</TooltipContent>
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button size="icon" variant="ghost" className="size-7" onClick={onRetry} aria-label="Transcribe again">
                      <RotateCcw />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Transcribe again</TooltipContent>
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button size="icon" variant="ghost" className="size-7" onClick={onRemove} aria-label="Remove">
                      <Trash2 />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Remove</TooltipContent>
                </Tooltip>
              </div>
              <span className="px-2 tabular-nums">
                {when(entry.at)}
                {entry.ms != null && ` · ${(entry.ms / 1000).toFixed(1)}s on ${entry.compute?.toUpperCase()}`}
              </span>
            </div>
          </>
        )}
      </div>
    </article>
  );
}
