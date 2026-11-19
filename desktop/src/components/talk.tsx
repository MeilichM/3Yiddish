import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowUp, Check, Copy, Download, MessageCircle, Mic, RotateCcw, SquarePen, X } from "lucide-react";
import { toast } from "sonner";
import { api, message, type ChatTurn } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Waveform } from "@/components/composer";
import { mb } from "@/components/model-setup";
import { duration } from "@/components/voice-note";
import { cn } from "@/lib/utils";

const SAVED = "3yiddish.talk";
/** Waveform history kept while recording. */
const BARS = 320;

interface Turn extends ChatTurn {
  at: number;
}

function loadTurns(): Turn[] {
  try {
    return JSON.parse(localStorage.getItem(SAVED) ?? "[]");
  } catch {
    return [];
  }
}

/** Space starts a spoken message unless the person is typing, or a dialog or menu is open. */
function typing(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  if (document.querySelector('[role="dialog"], [role="menu"]')) return true;
  return !!el && (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName));
}

/** First use: what chat is, how big, and the download. */
function ChatSetup({ onReady }: { onReady: () => void }) {
  const [size, setSize] = useState<number | null | undefined>(undefined);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.chatSize().then(setSize);
    const off = api.onChatProgress(setProgress);
    return () => void off.then((f) => f());
  }, []);

  async function download() {
    setBusy(true);
    setError(null);
    try {
      await api.chatDownload();
      onReady();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  const share = progress && progress.total ? (progress.done / progress.total) * 100 : 0;
  return (
    <div className="flex flex-1 items-center justify-center overflow-y-auto p-6">
      <div className="flex w-full max-w-md flex-col gap-5 rounded-2xl border bg-card p-6 text-card-foreground shadow-sm">
        <div className="flex flex-col gap-1.5">
          <h2 className="flex items-center gap-2 text-lg font-semibold tracking-tight">
            <MessageCircle className="size-5" /> Chat in Yiddish
          </h2>
          <p className="text-sm leading-relaxed text-muted-foreground">
            Talk or type, in Yiddish or English, and it answers in Yiddish. It runs on this
            computer like the rest of the app: a thinking model (Gemma, by Google) works out the
            answer, and our own translator, trained on heimish Yiddish, writes it the way people
            actually write.
          </p>
          <p className="text-sm leading-relaxed text-muted-foreground">
            With a graphics card an answer takes a few seconds; on the processor alone it can take
            a minute or more.
          </p>
        </div>

        {busy && progress ? (
          <div className="flex flex-col gap-2">
            <Progress value={share} />
            <div className="flex justify-end text-xs text-muted-foreground tabular-nums">
              {mb(progress.done)} of {mb(progress.total)}
            </div>
          </div>
        ) : (
          <Button onClick={download} disabled={busy} size="lg">
            <Download />
            {busy ? "Starting…" : size ? `Download chat (${(size / 1e9).toFixed(1)} GB)` : "Download chat"}
          </Button>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}
        <p className="text-xs text-muted-foreground">
          Downloaded once{size === null ? "" : ", checked after download"}. It is new and it makes
          mistakes: don't take its word on halacha.
        </p>
      </div>
    </div>
  );
}

function Bubble({ turn }: { turn: Turn }) {
  const [copied, setCopied] = useState(false);
  const mine = turn.role === "user";
  async function copy() {
    await navigator.clipboard.writeText(turn.content);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }
  return (
    <div className={cn("group flex flex-col gap-1", mine ? "items-end" : "items-start")}>
      <p
        dir={mine ? "auto" : "rtl"}
        className={cn(
          "max-w-[85%] rounded-2xl px-4 py-2.5 text-[1.0625rem] leading-relaxed whitespace-pre-wrap select-text",
          mine ? "bg-primary text-primary-foreground" : "border bg-card text-card-foreground",
        )}
      >
        {turn.content}
      </p>
      {!mine && (
        <Button
          size="icon"
          variant="ghost"
          className="size-7 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
          onClick={copy}
          aria-label="Copy"
        >
          {copied ? <Check /> : <Copy />}
        </Button>
      )}
    </div>
  );
}

/** Chat: talk to 3Yiddish and it answers in Yiddish. `canHear`: the speech model is here. */
export function TalkView({ canHear }: { canHear: boolean }) {
  const [ready, setReady] = useState<boolean | null>(null);
  const [turns, setTurns] = useState<Turn[]>(loadTurns);
  const [draft, setDraft] = useState("");
  /** Waiting for an answer since then. */
  const [waiting, setWaiting] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now());
  const [failed, setFailed] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const [hearing, setHearing] = useState(false);
  const [levels, setLevels] = useState<number[]>(() => Array(BARS).fill(0));
  const [seconds, setSeconds] = useState(0);
  /** The helpers were started in this run (the first answer starts them, which takes longer). */
  const started = useRef(false);
  const scroller = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    api.chatState().then((s) => setReady(s.ready));
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(SAVED, JSON.stringify(turns.slice(-200)));
    } catch {
      /* not kept */
    }
  }, [turns]);

  useEffect(() => {
    const el = scroller.current;
    el?.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [turns.length, waiting, failed, recording, hearing]);

  useEffect(() => {
    if (waiting == null) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [waiting]);

  const ask = useCallback(async (all: Turn[]) => {
    setFailed(null);
    setWaiting(Date.now());
    setNow(Date.now());
    try {
      const answer = await api.chatSend(all.map(({ role, content }) => ({ role, content })));
      started.current = true;
      setTurns((t) => [...t, { role: "assistant", content: answer, at: Date.now() }]);
    } catch (e) {
      setFailed(message(e));
    } finally {
      setWaiting(null);
    }
  }, []);

  const send = useCallback(
    (text: string) => {
      const content = text.trim();
      if (!content || waiting != null) return;
      const all: Turn[] = [...turns, { role: "user", content, at: Date.now() }];
      setTurns(all);
      setDraft("");
      void ask(all);
    },
    [turns, waiting, ask],
  );

  // ---- A spoken message: record, hear it, send it ----
  const start = useCallback(async () => {
    if (recording || hearing || waiting != null || !canHear) return;
    try {
      await api.recordStart();
      setLevels(Array(BARS).fill(0));
      setSeconds(0);
      setRecording(true);
    } catch (e) {
      toast.error(message(e));
    }
  }, [recording, hearing, waiting, canHear]);

  const stop = useCallback(async () => {
    if (!recording) return;
    setRecording(false);
    setHearing(true);
    try {
      const clip = await api.recordStop();
      const words = await api.chatHear(clip.path);
      if (words) send(words);
      else toast("Could not hear anything.");
    } catch (e) {
      toast(message(e));
    } finally {
      setHearing(false);
    }
  }, [recording, send]);

  const cancel = useCallback(() => {
    if (!recording) return;
    setRecording(false);
    void api.recordCancel();
  }, [recording]);

  useEffect(() => {
    if (!recording) return;
    const timer = setInterval(async () => {
      const l = await api.recordLevel();
      if (l.failed) {
        toast.error(l.failed);
        cancel();
        return;
      }
      setSeconds(l.seconds);
      setLevels((prev) => [...prev.slice(1), l.level]);
    }, 60);
    return () => clearInterval(timer);
  }, [recording, cancel]);

  // Hold Space to talk (when not typing); Esc cancels.
  useEffect(() => {
    if (!ready) return;
    const down = (e: KeyboardEvent) => {
      if ((e.code === "Space" || e.key === " ") && !e.repeat && !typing(e.target)) {
        e.preventDefault();
        void start();
      } else if (e.code === "Escape") cancel();
    };
    const up = (e: KeyboardEvent) => {
      if ((e.code === "Space" || e.key === " ") && !typing(e.target)) {
        e.preventDefault();
        void stop();
      }
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, [ready, start, stop, cancel]);

  if (ready === null) return <div className="flex-1" />;
  if (!ready) return <ChatSetup onReady={() => setReady(true)} />;

  const waited = waiting == null ? 0 : Math.floor((now - waiting) / 1000);
  const busy = waiting != null || hearing;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div ref={scroller} className="flex-1 overflow-y-auto overscroll-contain">
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 px-4 pt-6 pb-6">
          {turns.length === 0 && waiting == null && (
            <div className="flex flex-col items-center gap-3 py-20 text-center">
              <div className="flex size-12 items-center justify-center rounded-2xl border bg-card">
                <MessageCircle className="size-5" />
              </div>
              <h1 className="text-xl font-semibold tracking-tight">Ask something, in Yiddish or English</h1>
              <p className="max-w-sm text-sm text-muted-foreground">
                It answers in Yiddish. Type below, or hold Space to talk. Nothing leaves your computer.
              </p>
            </div>
          )}
          {turns.length > 0 && (
            <div className="flex justify-end">
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2 text-xs text-muted-foreground"
                disabled={busy}
                onClick={() => {
                  setTurns([]);
                  setFailed(null);
                }}
              >
                <SquarePen /> New conversation
              </Button>
            </div>
          )}
          {turns.map((turn, i) => (
            <Bubble key={`${turn.at}-${i}`} turn={turn} />
          ))}
          {waiting != null && (
            <div className="flex flex-col items-start gap-1.5">
              <div className="flex items-center gap-3 rounded-2xl border bg-card px-4 py-3 text-sm text-muted-foreground">
                <span>Thinking</span>
                <span className="flex gap-1">
                  {[0, 1, 2].map((i) => (
                    <span
                      key={i}
                      className="size-1.5 animate-bounce rounded-full bg-muted-foreground"
                      style={{ animationDelay: `${i * 150}ms` }}
                    />
                  ))}
                </span>
                <span className="tabular-nums">{waited}s</span>
              </div>
              {waited >= 5 && (
                <p className="px-1 text-xs text-muted-foreground">
                  {started.current
                    ? "On the processor alone an answer can take a minute or more."
                    : "The first answer takes longer: chat is starting up."}
                </p>
              )}
            </div>
          )}
          {failed && (
            <div className="flex items-center justify-between gap-3 rounded-2xl border px-4 py-3 text-sm">
              <span className="text-destructive">{failed}</span>
              <Button size="sm" variant="outline" onClick={() => ask(turns)}>
                <RotateCcw /> Try again
              </Button>
            </div>
          )}
        </div>
      </div>

      <div className="mx-auto w-full max-w-3xl px-4 pb-3">
        <div className="flex items-end gap-2 rounded-2xl border bg-card/60 p-2 shadow-lg shadow-black/5">
          {recording ? (
            <>
              <Button size="icon" variant="ghost" className="rounded-xl" onClick={cancel} aria-label="Cancel">
                <X />
              </Button>
              <div className="flex h-9 min-w-0 flex-1 items-center gap-3 px-1 text-sm">
                <span className="size-2 shrink-0 animate-pulse rounded-full bg-red-500" />
                <span className="w-10 shrink-0 tabular-nums">{duration(seconds)}</span>
                <Waveform levels={levels} />
              </div>
            </>
          ) : (
            <textarea
              ref={box}
              dir="auto"
              rows={1}
              value={draft}
              disabled={hearing}
              placeholder={hearing ? "Listening to what you said…" : "Type a message, or hold Space to talk"}
              onChange={(e) => {
                setDraft(e.target.value);
                e.target.style.height = "auto";
                e.target.style.height = `${Math.min(e.target.scrollHeight, 160)}px`;
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  send(draft);
                  if (box.current) box.current.style.height = "auto";
                }
              }}
              className="max-h-40 min-h-9 flex-1 resize-none bg-transparent px-2 py-1.5 text-[0.9375rem] leading-relaxed outline-none placeholder:text-muted-foreground"
            />
          )}
          {draft.trim() && !recording ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="icon"
                  className="size-9 rounded-xl"
                  disabled={busy}
                  onClick={() => send(draft)}
                  aria-label="Send"
                >
                  <ArrowUp />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Send (Enter)</TooltipContent>
            </Tooltip>
          ) : (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="icon"
                  className={cn("size-9 rounded-xl", recording && "bg-red-500 text-white hover:bg-red-600")}
                  disabled={!canHear || (busy && !recording)}
                  onClick={recording ? stop : start}
                  aria-label={recording ? "Stop and send" : "Talk"}
                >
                  {recording ? <ArrowUp /> : <Mic />}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{recording ? "Stop and send" : "Talk (or hold Space)"}</TooltipContent>
            </Tooltip>
          )}
        </div>
        <p className="pt-2 text-center text-xs text-muted-foreground">
          Chat is new and makes mistakes. Don't take its word on halacha.
        </p>
      </div>
    </div>
  );
}
