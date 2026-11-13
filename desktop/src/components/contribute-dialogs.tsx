import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
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

/** What sharing sends and what for: the core's words (src/share.rs `WHAT`). */
export const SHARE_WHAT =
  "What you transcribe here, your recordings and files with the text the app wrote, is sent to 3Yiddish with technical facts: app and model version, operating system, GPU or CPU, how long it took, and errors. No name or computer id goes with it. It is kept privately and used to test, and maybe later to improve, Yiddish speech to text.";
const SHARE_OFF = "You can switch it off any time in Settings.";

const STEPS = [
  "Say something in your own words. An idea is shown if you want one.",
  "The app writes down what it heard.",
  "Click any word it got wrong and fix it, or say that all of it is correct.",
  "Press Send when you have a few.",
];

/** What contributing means (the terminal app's text), with Never mind and I accept. */
export function ConsentDialog({
  open,
  onOpenChange,
  onAccept,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAccept: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="gap-5 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Help test Yiddish speech to text</DialogTitle>
          <DialogDescription>
            3Yiddish gets better only by being tested on real speech. You can help by recording
            yourself.
          </DialogDescription>
        </DialogHeader>
        <ol className="flex flex-col gap-2.5">
          {STEPS.map((step, i) => (
            <li key={i} className="flex gap-3 text-sm">
              <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium tabular-nums">
                {i + 1}
              </span>
              <span className="pt-px">{step}</span>
            </li>
          ))}
        </ol>
        <div className="flex flex-col gap-2 rounded-lg bg-muted/50 p-3 text-xs leading-relaxed text-muted-foreground">
          <p>
            Your takes stay on this computer until you press Send. What you send is used to test,
            and maybe later to improve, Yiddish speech to text, and may be published as a test set
            under any name you choose.
          </p>
          <p>
            Accepting also switches on sharing: what you transcribe in the rest of the app, your
            recordings and files with the text the app wrote and technical facts, is sent to
            3Yiddish, kept privately, and used to test and maybe later improve it. {SHARE_OFF}
          </p>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Never mind
          </Button>
          <Button onClick={onAccept}>I accept</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** "Share what you transcribe?": asked once. No thanks is the main button: the default is no. */
export function ShareDialog({
  open,
  onAnswer,
  onDismiss,
}: {
  open: boolean;
  onAnswer: (yes: boolean) => void;
  /** Closed without an answer: asked again next time. */
  onDismiss: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onDismiss()}>
      <DialogContent className="gap-5 sm:max-w-md" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>Share what you transcribe?</DialogTitle>
          <DialogDescription className="leading-relaxed">{SHARE_WHAT}</DialogDescription>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">{SHARE_OFF}</p>
        <DialogFooter>
          <Button variant="outline" onClick={() => onAnswer(true)}>
            Yes, share
          </Button>
          <Button onClick={() => onAnswer(false)}>No thanks</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Send: what it means, the name the takes go under, and Send or Not now. */
export function SendDialog({
  open,
  onOpenChange,
  count,
  name: savedName,
  sending,
  onSend,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  count: number;
  name: string;
  sending: boolean;
  onSend: (name: string) => void;
}) {
  const [name, setName] = useState(savedName);
  useEffect(() => {
    if (open) setName(savedName);
  }, [open, savedName]);
  const ok = name.trim().length > 0 && name.trim().length <= 40;

  return (
    <Dialog open={open} onOpenChange={(o) => !sending && onOpenChange(o)}>
      <DialogContent className="gap-5 sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            Send {count} take{count === 1 ? "" : "s"} to 3Yiddish?
          </DialogTitle>
          <DialogDescription className="leading-relaxed">
            The recordings and the text you fixed are used to test, and maybe later to improve,
            Yiddish speech to text. They may be published as a test set, with the name you give
            here. Any name will do.
          </DialogDescription>
        </DialogHeader>
        <form
          id="send"
          onSubmit={(e) => {
            e.preventDefault();
            if (ok && !sending) onSend(name.trim());
          }}
        >
          <Input
            autoFocus
            placeholder="Your name"
            aria-label="Your name"
            maxLength={40}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </form>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={sending}>
            Not now
          </Button>
          <Button type="submit" form="send" disabled={!ok || sending}>
            {sending && <Loader2 className="animate-spin" />}
            {sending ? "Sending" : "Send"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
