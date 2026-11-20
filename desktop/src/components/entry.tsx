import { Check, ChevronDown, Copy, FileAudio, RotateCcw, Send, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import type { Clip, Compute } from "@/lib/api";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
  /** The model that wrote the text (its version). */
  model?: string;
  error?: string;
}

function when(at: number) {
  return new Date(at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

export function EntryView({
  entry,
  models,
  activeModel,
  onRetry,
  onRemove,
  onContribute,
}: {
  entry: Entry;
  /** The installed models; with more than one, Transcribe again asks which. */
  models: { version: string; label: string }[];
  activeModel: string | null;
  /** Transcribe again, with this model's version; none: the active model. */
  onRetry: (model?: string) => void;
  onRemove: () => void;
  /** Open this one in Contribute, to fix and send (whether or not the Contribute switch is on). */
  onContribute?: () => void;
}) {
  const [copied, setCopied] = useState(false);
  /** The name of a model as the model menu gives it: its label, or its version without one. */
  const label = (version?: string) => models.find((m) => m.version === version)?.label ?? version;

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
            <Button size="sm" variant="outline" onClick={() => onRetry()}>
              <RotateCcw /> Try again
            </Button>
          </div>
        )}

        {entry.state === "done" && (
          <>
            <p
              // Yiddish reads right to left; the English note for an empty take, left to right.
              dir={entry.text ? "rtl" : "ltr"}
              onClick={copy}
              title="Click to copy"
              className="cursor-pointer px-4 pt-3 pb-1 text-[1.0625rem] leading-relaxed whitespace-pre-wrap select-text"
            >
              {entry.text || <span className="text-muted-foreground">Could not hear anything.</span>}
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
                {models.length > 1 ? (
                  // More than one model here: pick which one transcribes it again.
                  <DropdownMenu>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <DropdownMenuTrigger asChild>
                          <Button variant="ghost" className="h-7 gap-0.5 px-1.5" aria-label="Transcribe again with…">
                            <RotateCcw />
                            <ChevronDown className="size-3 opacity-60" />
                          </Button>
                        </DropdownMenuTrigger>
                      </TooltipTrigger>
                      <TooltipContent>Transcribe again with…</TooltipContent>
                    </Tooltip>
                    <DropdownMenuContent align="start" className="min-w-52">
                      <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
                        Transcribe again with
                      </DropdownMenuLabel>
                      {models.map((m) => (
                        <DropdownMenuItem key={m.version} onSelect={() => onRetry(m.version === activeModel ? undefined : m.version)}>
                          <span className="flex flex-col">
                            {m.label}
                            {m.label !== m.version && (
                              <span className="text-xs text-muted-foreground tabular-nums">{m.version}</span>
                            )}
                          </span>
                          <span className="ms-auto ps-4 text-xs text-muted-foreground">
                            {m.version === entry.model ? "wrote this" : m.version === activeModel ? "in use" : ""}
                          </span>
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuContent>
                  </DropdownMenu>
                ) : (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button size="icon" variant="ghost" className="size-7" onClick={() => onRetry()} aria-label="Transcribe again">
                        <RotateCcw />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Transcribe again</TooltipContent>
                  </Tooltip>
                )}
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button size="icon" variant="ghost" className="size-7" onClick={onRemove} aria-label="Remove">
                      <Trash2 />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Remove</TooltipContent>
                </Tooltip>
                {onContribute && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button size="icon" variant="ghost" className="size-7" onClick={onContribute} aria-label="Contribute this one">
                        <Send />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Contribute this one: fix it and send</TooltipContent>
                  </Tooltip>
                )}
              </div>
              {/* The time always, in the corner; how it was made next to it, only on hover. */}
              <span className="flex items-center gap-1.5 px-2 tabular-nums">
                {entry.ms != null && (
                  <span className="opacity-0 transition-opacity group-hover:opacity-100">
                    {models.length > 1 && entry.model && <>{label(entry.model)} · </>}
                    {(entry.ms / 1000).toFixed(1)}s on {entry.compute?.toUpperCase()} ·
                  </span>
                )}
                {when(entry.at)}
              </span>
            </div>
          </>
        )}
      </div>
    </article>
  );
}
