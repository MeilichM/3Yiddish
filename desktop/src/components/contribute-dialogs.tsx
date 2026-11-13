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
