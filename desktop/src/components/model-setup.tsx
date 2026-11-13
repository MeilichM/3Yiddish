import { Download, ShieldCheck } from "lucide-react";
import type { Progress as Downloading } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";

function mb(bytes: number) {
  return `${Math.round(bytes / 1024 / 1024)} MB`;
}

/** First run: the speech model is not on this computer yet. */
export function ModelSetup({
  progress,
  busy,
  error,
  onDownload,
}: {
  progress: Downloading | null;
  busy: boolean;
  error: string | null;
  onDownload: () => void;
}) {
  const share = progress && progress.total ? (progress.done / progress.total) * 100 : 0;
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <div className="flex w-full max-w-md flex-col gap-5 rounded-2xl border bg-card p-6 text-card-foreground shadow-sm">
        <div className="flex flex-col gap-1.5">
          <h2 className="text-lg font-semibold tracking-tight">One more step: the speech model</h2>
          <p className="text-sm text-muted-foreground">
            3Yiddish turns your speech into text right here on your computer. For that it needs its
            speech model, about 575 MB, downloaded once.
          </p>
        </div>

        {busy && progress ? (
          <div className="flex flex-col gap-2">
            <Progress value={share} />
            <div className="flex justify-between text-xs text-muted-foreground tabular-nums">
              <span className="truncate">{progress.file}</span>
              <span>
                {mb(progress.done)} of {mb(progress.total)}
              </span>
            </div>
          </div>
        ) : (
          <Button onClick={onDownload} disabled={busy} size="lg">
            <Download /> {busy ? "Starting…" : "Download the model"}
          </Button>
        )}

        {error && <p className="text-sm text-destructive">{error}</p>}

        <p className="flex items-start gap-2 text-xs text-muted-foreground">
          <ShieldCheck className="mt-px size-4 shrink-0" />
          It is checked after download. After that, your recordings never leave this computer.
        </p>
      </div>
    </div>
  );
}
