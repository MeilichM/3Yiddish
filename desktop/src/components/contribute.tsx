import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowUp, Check, ChevronLeft, List, Loader2, Mic, Pause, Play, RotateCcw, Send, Shuffle, X } from "lucide-react";
import { toast } from "sonner";
import { api, message, type Clip, type ContributeState, type Mark, type Take } from "@/lib/api";
import { COLOR, LABEL, REASONS, STATUS, applyMarks, clashes, sortMarks, squash } from "@/lib/marks";
import { SendDialog } from "@/components/contribute-dialogs";
import { VoiceNote, duration } from "@/components/voice-note";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Progress as Bar } from "@/components/ui/progress";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

type Page = "record" | "review" | "list";
type Span = { start: number; end: number };
const BARS = 40;
const MAX_SECONDS = 180;
const reviewed = (t: Take) => t.status === "correct" || t.status === "fixed";

/** Typing in a field, or a dialog is open: keys are not shortcuts then. */
function busyKeys(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  if (document.querySelector('[role="dialog"], [role="menu"]')) return true;
  return !!el && (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName));
}

function StatusBadge({ status }: { status: Take["status"] }) {
  return (
    <span
      className={cn(
        "rounded-full px-2 py-0.5 text-[11px] font-medium",
        status === "pending" && "bg-amber-500/15 text-amber-600 dark:text-amber-400",
        status === "correct" && "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
        status === "fixed" && "bg-blue-500/15 text-blue-600 dark:text-blue-400",
        status === "tossed" && "bg-muted text-muted-foreground",
      )}
    >
      {STATUS[status]}
    </span>
  );
}

/**
 * Contribute: the terminal app's playground in the window. Record a phrase in your own words,
 * read what the model wrote, mark what it got wrong, and Send the takes when you have a few.
 */
export function ContributeView({ ready }: { ready: boolean }) {
  const [state, setState] = useState<ContributeState | null>(null);
  const [page, setPage] = useState<Page>("record");

  // Record
  const [recording, setRecording] = useState(false);
  const [levels, setLevels] = useState<number[]>(() => Array(BARS).fill(0));
  const [seconds, setSeconds] = useState(0);
  const [thinking, setThinking] = useState(false);
  const [retry, setRetry] = useState(false);
  const busy = useRef(false);

  // Review
  const [take, setTake] = useState<Take | null>(null);
  const [clip, setClip] = useState<Clip | null>(null);
  const [marks, setMarks] = useState<Mark[]>([]);
  const [typed, setTyped] = useState<string | null>(null);
  const [fromList, setFromList] = useState(false);
  const [sel, setSel] = useState<Span | null>(null);
  const [editing, setEditing] = useState(-1);
  const [reason, setReason] = useState("wrong");
  const [touched, setTouched] = useState(false);
  const [right, setRight] = useState("");
  const [note, setNote] = useState("");
  const [help, setHelp] = useState<{ text: string; bad: boolean }>({ text: "", bad: false });
  const [saving, setSaving] = useState(false);
  const [askRedo, setAskRedo] = useState(false);

  // Send
  const [sendOpen, setSendOpen] = useState(false);
  const [sending, setSending] = useState(false);

  const items = state?.items ?? [];
  const goal = state?.goal ?? 0;
  const done = items.filter(reviewed).length;
  const unsent = state?.canSend ? items.filter((t) => reviewed(t) && !t.sent).length : 0;

  const load = useCallback(async () => {
    const s = await api.contributeState();
    setState(s);
    return s;
  }, []);

  const openReview = useCallback((t: Take, list: boolean) => {
    setTake(t);
    setMarks(t.marks.map((m) => ({ ...m })));
    setTyped(t.typed ? t.final : null);
    setFromList(list);
    setSel(null);
    setEditing(-1);
    setHelp({ text: "", bad: false });
    setPage("review");
    setClip(null);
    api
      .takeClip(t.n)
      .then(setClip)
      .catch((e) => toast.error(message(e)));
  }, []);

  // A take that was recorded but not reviewed comes back.
  useEffect(() => {
    load()
      .then((s) => {
        const waiting = [...s.items].reverse().find((t) => t.status === "pending");
        if (waiting) openReview(waiting, false);
      })
      .catch((e) => toast.error(message(e)));
  }, [load, openReview]);

  // Leaving Contribute while recording: the take is dropped.
  const recordingRef = useRef(false);
  recordingRef.current = recording;
  useEffect(() => () => void (recordingRef.current && api.recordCancel()), []);

  const setIdea = (idea: string) => setState((s) => (s ? { ...s, idea } : s));
  const nextIdea = () => api.ideaNext().then(setIdea);

  // ---- Record ----

  const gotTake = useCallback(
    (t: Take) => {
      setState((s) => (s ? { ...s, items: [...s.items.filter((i) => i.n !== t.n), t] } : s));
      openReview(t, false);
    },
    [openReview],
  );

  const draft = useCallback(
    async (job: () => Promise<Take>) => {
      setThinking(true);
      setRetry(false);
      try {
        gotTake(await job());
      } catch (e) {
        const why = message(e);
        toast.error(why);
        // A draft that failed can be tried again without recording again.
        setRetry(!/too short|too long|not recording/i.test(why));
      } finally {
        setThinking(false);
      }
    },
    [gotTake],
  );

  const startRecording = useCallback(async () => {
    if (busy.current || recording || thinking || !ready) return;
    busy.current = true;
    try {
      await api.recordStart();
      setRetry(false);
      setLevels(Array(BARS).fill(0));
      setSeconds(0);
      setRecording(true);
    } catch (e) {
      toast.error(message(e));
    } finally {
      busy.current = false;
    }
  }, [recording, thinking, ready]);

  const stopRecording = useCallback(async () => {
    if (busy.current || !recording) return;
    busy.current = true;
    setRecording(false);
    try {
      await draft(api.takeFinish);
    } finally {
      busy.current = false;
    }
  }, [recording, draft]);

  const cancelRecording = useCallback(() => {
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
        cancelRecording();
        return;
      }
      setSeconds(l.seconds);
      setLevels((prev) => [...prev.slice(1), l.level]);
      if (l.seconds >= MAX_SECONDS) void stopRecording();
    }, 60);
    return () => clearInterval(timer);
  }, [recording, cancelRecording, stopRecording]);

  // ---- Review ----

  const words = take?.words ?? [];
  const heardOf = (span: Span | null) => (span ? words.slice(span.start, span.end).join(" ") : "");
  const finalText = typed != null ? squash(typed) : applyMarks(words, marks);
  const changed = marks.length > 0 || typed != null;

  function defaultHelp(span: Span, why: string) {
    if (span.start === span.end) return "";
    if (why === "weird") return "Leave it empty if nothing was said there.";
    if (why === "fluency") return "Optional: leave it empty to only flag the spot.";
    return "";
  }

  /** The panel for one mark. `k`: the saved mark being edited (-1: new); `keep`: the selection only grew. */
  function openSheet(span: Span, k: number, keep = false) {
    const m = k >= 0 ? marks[k] : null;
    const place = span.start === span.end;
    let why = reason;
    if (!keep) {
      why = m ? m.reason : place ? "dropped" : "wrong";
      setReason(why);
      setTouched(!!m);
      setRight(m ? (m.right ?? "") : place ? "" : heardOf(span));
      setNote(m ? m.note : "");
    } else if (!touched) {
      setRight(why === "weird" ? "" : heardOf(span));
    }
    setSel(span);
    setEditing(k);
    setHelp({ text: defaultHelp(span, why), bad: false });
  }
  function closeSheet() {
    setSel(null);
    setEditing(-1);
    setHelp({ text: "", bad: false });
  }
  const bad = (text: string) => setHelp({ text, bad: true });

  function tapWord(i: number) {
    const k = marks.findIndex((m) => m.start <= i && i < m.end);
    if (k >= 0) return openSheet({ start: marks[k].start, end: marks[k].end }, k);
    if (sel && editing < 0 && sel.end > sel.start) {
      if (sel.end - sel.start === 1 && sel.start === i) return closeSheet();
      const next = { start: Math.min(sel.start, i), end: Math.max(sel.end, i + 1) };
      if (clashes(marks, next.start, next.end, -1))
        return bad("That would run into another mark. Mark up to it, or remove it first.");
      return openSheet(next, -1, true);
    }
    openSheet({ start: i, end: i + 1 }, -1);
  }

  function pickReason(why: string) {
    setReason(why);
    if (!touched && sel) setRight(why === "weird" ? "" : heardOf(sel));
    if (sel) setHelp({ text: defaultHelp(sel, why), bad: false });
  }

  function saveMark() {
    if (!sel) return;
    const place = sel.start === sel.end;
    const heard = heardOf(sel);
    let fix: string | null = squash(right);
    const why = note.trim();
    if (place) {
      if (!fix) return bad("Type the missing word.");
    } else if (reason === "fluency") {
      if (!fix) fix = null;
    } else if (reason !== "weird") {
      if (!fix) return bad("Type what it should be.");
      if (reason === "wrong" && fix === heard)
        return bad("That is what was written. Change it, or pick another reason.");
    }
    if (!place && reason === "custom" && !why) return bad("Say the reason in a few words.");
    const mark: Mark = {
      start: sel.start,
      end: sel.end,
      heard,
      right: fix,
      reason: place ? "dropped" : reason,
      note: !place && reason === "custom" ? why : "",
    };
    setMarks((all) => sortMarks(editing >= 0 ? all.map((m, k) => (k === editing ? mark : m)) : [...all, mark]));
    closeSheet();
  }

  function addPlace(p: number) {
    const existing = marks.findIndex((m) => m.start === p && m.end === p);
    if (existing >= 0) return openSheet({ start: p, end: p }, existing);
    if (clashes(marks, p, p, -1)) return bad("That spot is inside another mark.");
    openSheet({ start: p, end: p }, -1);
  }

  function leaveReview(list = fromList) {
    setTake(null);
    setClip(null);
    closeSheet();
    setPage(list ? "list" : "record");
  }

  async function save(toss: boolean) {
    if (!take || saving) return;
    if (!toss && typed != null && !typed.trim()) return toast.error("Type what you said, or toss the take.");
    setSaving(true);
    try {
      const saved = await api.takeReview(take.n, {
        marks: marks.map(({ start, end, right, reason, note }) => ({ start, end, right, reason, note })),
        typed: typed != null ? typed : null,
        toss,
      });
      setState((s) => (s ? { ...s, items: s.items.map((i) => (i.n === saved.n ? saved : i)) } : s));
      if (!fromList) void nextIdea();
      leaveReview();
    } catch (e) {
      toast.error(message(e));
    } finally {
      setSaving(false);
    }
  }

  async function redo() {
    if (!take || saving) return;
    setAskRedo(false);
    setSaving(true);
    try {
      await api.takeDelete(take.n);
      setState((s) => (s ? { ...s, items: s.items.filter((i) => i.n !== take.n) } : s));
      leaveReview(false);
    } catch (e) {
      toast.error(message(e));
    } finally {
      setSaving(false);
    }
  }

  function typeOut() {
    closeSheet();
    setTyped((t) => (t == null ? applyMarks(words, marks) : null));
  }

  // ---- Send ----

  async function send(name: string) {
    setSending(true);
    try {
      const { sent, left } = await api.sendTakes(name);
      setSendOpen(false);
      toast.success(
        left > 0
          ? `Sent ${sent} takes. Thank you! Send again for the other ${left}.`
          : `Sent ${sent} take${sent === 1 ? "" : "s"}. Thank you!`,
      );
    } catch (e) {
      toast.error(message(e));
    } finally {
      setSending(false);
      void load();
    }
  }

  // ---- Keys: Space records on the Record screen; Esc closes the mark panel ----

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.code === "Escape") {
        if (recording) cancelRecording();
        else if (sel && !document.querySelector('[role="dialog"]')) closeSheet();
        return;
      }
      if ((e.code !== "Space" && e.key !== " ") || e.repeat || busyKeys(e.target) || page !== "record") return;
      e.preventDefault();
      void (recording ? stopRecording() : startRecording());
    };
    window.addEventListener("keydown", down);
    return () => window.removeEventListener("keydown", down);
  });

  if (!state) return <div className="flex-1" />;

  const nextNumber = done + 1;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* The view's own bar: where you are, all takes, and Send. */}
      <div className="mx-auto flex w-full max-w-2xl items-center gap-2 px-4 pt-3 pb-1">
        {page === "record" ? (
          <span className="text-sm text-muted-foreground">
            {goal ? `${done} of ${goal} done` : done ? `${done} done` : "Contribute"}
          </span>
        ) : (
          <Button variant="ghost" size="sm" className="-ml-2" onClick={() => (page === "review" ? leaveReview() : setPage("record"))}>
            <ChevronLeft /> Back
          </Button>
        )}
        <div className="ml-auto flex items-center gap-1">
          {page !== "list" && (
            <Button variant="ghost" size="sm" onClick={() => leaveReview(true)} disabled={recording || thinking}>
              <List /> All takes
              {items.length > 0 && <span className="text-muted-foreground tabular-nums">{items.length}</span>}
            </Button>
          )}
          {unsent > 0 && (
            <Button size="sm" onClick={() => setSendOpen(true)} disabled={recording || thinking}>
              <Send /> Send {unsent} take{unsent === 1 ? "" : "s"}
            </Button>
          )}
        </div>
      </div>
      {goal > 0 && <Bar value={Math.min(100, (done / goal) * 100)} className="mx-auto h-1 w-full max-w-2xl px-4" />}

      {page === "record" && (
        <main className="mx-auto flex w-full max-w-2xl flex-1 flex-col items-center justify-center gap-8 overflow-y-auto px-4 py-8">
          <div className="text-center">
            <h1 className="text-4xl font-semibold tracking-tight tabular-nums">Take {nextNumber}</h1>
            {goal > 0 && (
              <p className="mt-1 text-sm text-muted-foreground">
                {done >= goal ? `The set of ${goal} is complete. More is fine.` : `of ${goal}`}
              </p>
            )}
          </div>

          {state.idea && (
            <div className="flex w-full items-center gap-3 rounded-xl border bg-card p-4">
              <div className="min-w-0 flex-1">
                <p className="text-xs text-muted-foreground">An idea, if you want one</p>
                <p dir="rtl" lang="yi" className="mt-1 text-xl leading-relaxed">
                  {state.idea}
                </p>
              </div>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon" onClick={nextIdea} disabled={recording} aria-label="Another idea">
                    <Shuffle />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Another idea</TooltipContent>
              </Tooltip>
            </div>
          )}

          <div className="flex h-24 w-full flex-col items-center justify-center gap-3 text-center">
            {recording ? (
              <>
                <span className="text-3xl font-semibold text-red-500 tabular-nums">{duration(seconds)}</span>
                <div className="flex h-12 w-full max-w-md items-center justify-center gap-[3px]">
                  {levels.map((l, i) => (
                    <span
                      key={i}
                      className="w-1 rounded-full bg-red-500/80"
                      style={{ height: `${Math.max(8, Math.min(100, l * 140))}%` }}
                    />
                  ))}
                </div>
              </>
            ) : thinking ? (
              <p className="flex items-center gap-2 text-sm">
                <Loader2 className="size-4 animate-spin" /> Writing down what you said
              </p>
            ) : retry ? (
              <Button variant="outline" onClick={() => draft(api.takeRetry)}>
                <RotateCcw /> Send it again
              </Button>
            ) : (
              <p className="text-sm text-muted-foreground">
                {ready ? "Click, and say it in your own words." : "Waiting for the speech model…"}
              </p>
            )}
          </div>

          <div className="flex flex-col items-center gap-3">
            <button
              type="button"
              onClick={recording ? stopRecording : startRecording}
              disabled={thinking || (!ready && !recording)}
              aria-label={recording ? "Stop" : "Record"}
              className={cn(
                "flex size-20 items-center justify-center rounded-full transition-all outline-none focus-visible:ring-4 focus-visible:ring-ring/40 disabled:opacity-40",
                recording
                  ? "bg-red-500 text-white ring-8 ring-red-500/20"
                  : "bg-primary text-primary-foreground hover:scale-105",
              )}
            >
              {recording ? <ArrowUp className="size-8" /> : <Mic className="size-8" />}
            </button>
            <p className="text-xs text-muted-foreground">
              {recording ? "Click or press Space to finish · Esc cancels" : "or press Space"}
            </p>
          </div>
        </main>
      )}

      {page === "review" && take && (
        <>
          <main className="flex-1 overflow-y-auto">
            <div className="mx-auto flex w-full max-w-2xl flex-col gap-4 px-4 py-4">
              <div className="flex items-center gap-2">
                {clip ? (
                  <VoiceNote key={clip.id} clip={clip} className="max-w-none flex-1" />
                ) : (
                  <div className="h-12 flex-1 animate-pulse rounded-2xl bg-muted" />
                )}
                <Button
                  variant="outline"
                  onClick={() => (take.status === "pending" ? redo() : setAskRedo(true))}
                  disabled={saving}
                >
                  <RotateCcw /> Redo
                </Button>
              </div>

              <section className="rounded-xl border bg-card">
                <div className="flex items-center justify-between px-4 pt-3 text-sm text-muted-foreground">
                  <span>Take {take.n}</span>
                  <StatusBadge status={take.status} />
                </div>
                {typed != null ? (
                  <textarea
                    autoFocus
                    dir="rtl"
                    lang="yi"
                    rows={4}
                    aria-label="The whole text"
                    value={typed}
                    onChange={(e) => setTyped(e.target.value)}
                    className="mx-4 my-3 block w-[calc(100%-2rem)] resize-y rounded-lg border bg-transparent px-3 py-2 text-xl leading-relaxed outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
                  />
                ) : (
                  <Words
                    words={words}
                    marks={marks}
                    sel={sel}
                    editing={editing}
                    onWord={tapWord}
                    onMark={(k) => openSheet({ start: marks[k].start, end: marks[k].end }, k)}
                  />
                )}
                <p className={cn("px-4 pb-3 text-xs", help.bad && !sel ? "text-destructive" : "text-muted-foreground")}>
                  {help.bad && !sel
                    ? help.text
                    : typed != null
                      ? "Type exactly what you said."
                      : words.length
                        ? "Click a word that is wrong. Click a second word to mark everything between them."
                        : "Nothing was heard. Type what you said, or toss the take."}
                </p>
              </section>

              {sel && (
                <MarkPanel
                  words={words}
                  sel={sel}
                  editing={editing}
                  reason={reason}
                  right={right}
                  note={note}
                  help={help}
                  onReason={pickReason}
                  onRight={(v) => {
                    setRight(v);
                    setTouched(true);
                    setHelp((h) => ({ ...h, bad: false }));
                  }}
                  onNote={setNote}
                  onClose={closeSheet}
                  onSave={saveMark}
                  onRemove={() => {
                    setMarks((all) => all.filter((_, k) => k !== editing));
                    closeSheet();
                  }}
                  onBefore={() => addPlace(sel.start)}
                  onAfter={() => addPlace(sel.end)}
                />
              )}

              <section className="rounded-xl border px-4 py-3">
                <p className="text-xs text-muted-foreground">Saved as</p>
                <p dir="rtl" lang="yi" className={cn("mt-1 text-lg leading-relaxed", !changed && "text-muted-foreground")}>
                  {finalText || "(nothing)"}
                </p>
              </section>
            </div>
          </main>

          <footer className="border-t">
            <div className="mx-auto flex w-full max-w-2xl items-center gap-2 px-4 py-3">
              <Button variant="ghost" onClick={typeOut} disabled={saving}>
                {typed != null ? "Back to marking words" : "Type it out instead"}
              </Button>
              <Button variant="ghost" className="ml-auto text-destructive hover:text-destructive" onClick={() => save(true)} disabled={saving}>
                Toss
              </Button>
              <Button
                size="lg"
                className={cn("min-w-32", !changed && "bg-emerald-600 text-white hover:bg-emerald-600/90")}
                onClick={() => save(false)}
                disabled={saving || !!sel}
              >
                {saving ? <Loader2 className="animate-spin" /> : !changed && <Check />}
                {changed ? (fromList ? "Save" : "Save & next") : "All correct"}
              </Button>
            </div>
          </footer>
        </>
      )}

      {page === "list" && (
        <TakeList
          items={items}
          goal={goal}
          onOpen={(t) => openReview(t, true)}
          onRecord={() => setPage("record")}
        />
      )}

      <Dialog open={askRedo} onOpenChange={setAskRedo}>
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>Record take {take?.n} again?</DialogTitle>
            <DialogDescription>The recording you have now is deleted.</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAskRedo(false)}>
              Keep it
            </Button>
            <Button variant="destructive" onClick={redo}>
              Record again
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <SendDialog
        open={sendOpen}
        onOpenChange={setSendOpen}
        count={unsent}
        name={state.name}
        sending={sending}
        onSend={send}
      />
    </div>
  );
}

/** The draft's words, each one clickable, with the marks drawn over them. */
function Words({
  words,
  marks,
  sel,
  editing,
  onWord,
  onMark,
}: {
  words: string[];
  marks: Mark[];
  sel: Span | null;
  editing: number;
  onWord: (i: number) => void;
  onMark: (k: number) => void;
}) {
  const word = (i: number) => (
    <button
      key={`w${i}`}
      type="button"
      onClick={() => onWord(i)}
      className={cn(
        "rounded-lg px-1.5 py-0.5 transition-colors hover:bg-muted",
        sel && editing < 0 && sel.start <= i && i < sel.end && "bg-primary text-primary-foreground hover:bg-primary",
      )}
    >
      {words[i]}
    </button>
  );
  const out: React.ReactNode[] = [];
  for (let p = 0; p <= words.length; ) {
    // A place between two words: a missing word, saved or being added.
    const place = marks.findIndex((m) => m.start === p && m.end === p);
    const adding = sel && sel.start === p && sel.end === p;
    if (place >= 0 || adding) {
      out.push(
        <button
          key={`p${p}`}
          type="button"
          style={{ "--c": COLOR.dropped } as React.CSSProperties}
          onClick={() => place >= 0 && onMark(place)}
          className={cn(
            "inline-flex flex-col items-center rounded-lg border-2 border-dashed border-(--c) px-2 py-0.5 leading-tight text-(--c)",
            (adding || editing === place) && "bg-[color-mix(in_srgb,var(--c)_15%,transparent)]",
          )}
        >
          <span>{place >= 0 ? marks[place].right : "+"}</span>
          <small dir="ltr" className="text-[10px] font-medium tracking-wide uppercase">
            missing
          </small>
        </button>,
      );
    }
    if (p === words.length) break;
    const k = marks.findIndex((m) => m.start === p && m.end > p);
    if (k < 0) {
      out.push(word(p));
      p++;
      continue;
    }
    const m = marks[k];
    const heard = words.slice(m.start, m.end).join(" ");
    const changed = m.right != null && m.right !== heard;
    const heardWords: React.ReactNode[] = [];
    for (let i = m.start; i < m.end; i++) heardWords.push(word(i));
    out.push(
      <span
        key={`m${k}`}
        style={{ "--c": COLOR[m.reason] ?? COLOR.custom } as React.CSSProperties}
        className={cn(
          "inline-flex flex-col items-stretch rounded-xl bg-[color-mix(in_srgb,var(--c)_12%,transparent)] ring-1 ring-[color-mix(in_srgb,var(--c)_50%,transparent)] ring-inset",
          editing === k && "ring-2 ring-(--c)",
        )}
      >
        <span
          className={cn(
            "flex flex-wrap justify-center",
            changed && "[&>button]:text-muted-foreground [&>button]:line-through [&>button]:decoration-(--c) [&>button]:decoration-2",
          )}
        >
          {heardWords}
        </span>
        {changed && m.right && (
          <span className="px-2 text-center text-lg leading-snug font-semibold text-(--c)">{m.right}</span>
        )}
        <span dir="ltr" className="px-2 pb-1 text-center text-[10px] font-medium tracking-wide text-(--c) uppercase">
          {changed && !m.right ? "not said" : m.reason === "custom" ? m.note : LABEL[m.reason]}
        </span>
      </span>,
    );
    p = m.end;
  }
  return (
    <div dir="rtl" lang="yi" className="flex flex-wrap items-start gap-x-0.5 gap-y-2 px-3 py-3 text-2xl leading-snug">
      {out}
    </div>
  );
}

/** One mark: why the words are wrong and what they should be. */
function MarkPanel({
  words,
  sel,
  editing,
  reason,
  right,
  note,
  help,
  onReason,
  onRight,
  onNote,
  onClose,
  onSave,
  onRemove,
  onBefore,
  onAfter,
}: {
  words: string[];
  sel: Span;
  editing: number;
  reason: string;
  right: string;
  note: string;
  help: { text: string; bad: boolean };
  onReason: (reason: string) => void;
  onRight: (value: string) => void;
  onNote: (value: string) => void;
  onClose: () => void;
  onSave: () => void;
  onRemove: () => void;
  onBefore: () => void;
  onAfter: () => void;
}) {
  const place = sel.start === sel.end;
  const shown = place
    ? [words[sel.start - 1], "[ + ]", words[sel.start]].filter(Boolean).join(" ")
    : words.slice(sel.start, sel.end).join(" ");
  const enter = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      onSave();
    }
  };

  return (
    <section className="flex flex-col gap-4 rounded-xl border bg-card p-4 shadow-sm">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-xs text-muted-foreground">{place ? "A word is missing here" : "You marked"}</p>
          <p dir="rtl" lang="yi" className="mt-1 text-xl">
            {shown}
          </p>
        </div>
        <Button variant="ghost" size="icon-sm" onClick={onClose} aria-label="Deselect">
          <X />
        </Button>
      </div>

      {!place && (
        <div className="flex flex-wrap gap-1.5">
          {REASONS.map(([key, label]) => (
            <button
              key={key}
              type="button"
              style={{ "--c": COLOR[key] } as React.CSSProperties}
              onClick={() => onReason(key)}
              className={cn(
                "flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-xs font-medium transition-colors hover:bg-muted",
                key === reason && "border-(--c) bg-[color-mix(in_srgb,var(--c)_14%,transparent)] hover:bg-[color-mix(in_srgb,var(--c)_14%,transparent)]",
              )}
            >
              <span className="size-2 rounded-full bg-(--c)" />
              {label}
            </button>
          ))}
        </div>
      )}

      {!place && reason === "custom" && (
        <Input
          autoFocus
          maxLength={200}
          placeholder="Your reason, in a few words"
          value={note}
          onChange={(e) => onNote(e.target.value)}
          onKeyDown={enter}
        />
      )}

      <div className="flex flex-col gap-1.5">
        <label htmlFor="right" className="text-xs text-muted-foreground">
          {place ? "The missing word or words" : "What it should be"}
        </label>
        <Input
          id="right"
          autoFocus={reason !== "custom"}
          dir="rtl"
          lang="yi"
          autoComplete="off"
          spellCheck={false}
          className="h-10 text-lg"
          value={right}
          onChange={(e) => onRight(e.target.value)}
          onKeyDown={enter}
        />
        {help.text && (
          <p className={cn("text-xs", help.bad ? "text-destructive" : "text-muted-foreground")}>{help.text}</p>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {!place && editing < 0 && (
          <>
            <Button variant="outline" size="sm" onClick={onBefore}>
              Add word before
            </Button>
            <Button variant="outline" size="sm" onClick={onAfter}>
              Add word after
            </Button>
          </>
        )}
        <div className="ml-auto flex items-center gap-2">
          {editing >= 0 && (
            <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" onClick={onRemove}>
              Remove
            </Button>
          )}
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" onClick={onSave}>
            {editing >= 0 ? "Update mark" : "Save mark"}
          </Button>
        </div>
      </div>
    </section>
  );
}

/** All takes, newest first: what each became, its status, and its recording. */
function TakeList({
  items,
  goal,
  onOpen,
  onRecord,
}: {
  items: Take[];
  goal: number;
  onOpen: (t: Take) => void;
  onRecord: () => void;
}) {
  const done = items.filter(reviewed).length;
  const waiting = items.filter((t) => t.status === "pending").length;
  const tossed = items.filter((t) => t.status === "tossed").length;
  const [playing, setPlaying] = useState<number | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);

  useEffect(() => () => audio.current?.pause(), []);

  async function play(t: Take) {
    if (playing === t.n) {
      audio.current?.pause();
      setPlaying(null);
      return;
    }
    try {
      audio.current?.pause();
      const clip = await api.takeClip(t.n);
      const el = new Audio(await api.audioUrl(clip.path));
      el.onended = () => setPlaying(null);
      audio.current = el;
      setPlaying(t.n);
      await el.play();
    } catch (e) {
      setPlaying(null);
      toast.error(message(e));
    }
  }

  const summary = [
    goal ? (done >= goal ? `The set of ${goal} is complete. ${done} done` : `${done} of ${goal} done`) : `${done} done`,
    waiting ? `${waiting} to review` : "",
    tossed ? `${tossed} tossed` : "",
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <>
      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-2xl flex-col gap-3 px-4 py-4">
          <p className="text-sm text-muted-foreground">{summary}</p>
          {items.length === 0 ? (
            <p className="py-16 text-center text-sm text-muted-foreground">No takes yet. Record the first one.</p>
          ) : (
            <div className="divide-y rounded-xl border">
              {[...items].reverse().map((t) => (
                <div
                  key={t.n}
                  role="button"
                  tabIndex={0}
                  onClick={() => onOpen(t)}
                  onKeyDown={(e) => e.key === "Enter" && onOpen(t)}
                  className="flex cursor-pointer items-center gap-3 px-3 py-2.5 transition-colors outline-none first:rounded-t-xl last:rounded-b-xl hover:bg-muted/50 focus-visible:bg-muted/50"
                >
                  <span className="w-7 shrink-0 text-sm text-muted-foreground tabular-nums">{t.n}</span>
                  <div className="flex min-w-0 flex-1 flex-col items-start gap-1">
                    <span
                      dir="rtl"
                      lang="yi"
                      className={cn(
                        "w-full truncate text-right text-base",
                        t.status === "tossed" && "text-muted-foreground line-through",
                      )}
                    >
                      {t.final || t.draft || "(nothing heard)"}
                    </span>
                    <span className="flex items-center gap-2">
                      <StatusBadge status={t.status} />
                      {t.sent && <span className="text-[11px] text-muted-foreground">Sent</span>}
                    </span>
                  </div>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="shrink-0 rounded-full"
                    aria-label={`Play take ${t.n}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      void play(t);
                    }}
                  >
                    {playing === t.n ? <Pause className="fill-current" /> : <Play className="fill-current" />}
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>
      </main>
      <footer className="border-t">
        <div className="mx-auto flex w-full max-w-2xl justify-end px-4 py-3">
          <Button size="lg" onClick={onRecord}>
            <Mic /> Record a take
          </Button>
        </div>
      </footer>
    </>
  );
}
