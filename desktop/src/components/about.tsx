/** What 3Yiddish is built on (the repository's NOTICE, as of 0.6.0), and each one's license. */
const CREDITS: [name: string, license: string][] = [
  ["3Yiddish, © 2026 TripleU613", "CC BY-NC-SA 4.0"],
  ["OpenAI Whisper large-v3-turbo, © 2022 OpenAI (the Whisper-based model)", "MIT"],
  ["Meta Omnilingual ASR (CTC 1B v2), © 2025 Meta Platforms, Inc. (the Omnilingual-based model)", "Apache 2.0"],
  ["ivrit-ai/yi-whisper-large-v3-turbo, by ivrit.ai (models up to 2026.11.11)", "Apache 2.0"],
  ["whisper.cpp (speech engine)", "MIT"],
  ["ONNX Runtime, © Microsoft Corporation (runs the Omnilingual-based model)", "MIT"],
  ["Gemma 4 E4B, by Google (chat's thinking model, downloaded with chat)", "Gemma Terms of Use"],
  ["Our translator, a fine-tune of Meta's NLLB-200 (distilled 1.3B), © Meta Platforms, Inc. (chat)", "CC BY-NC 4.0"],
  ["llama.cpp's server program, © 2023-2026 The ggml authors (runs chat's thinking model)", "MIT"],
  ["Tauri", "MIT / Apache 2.0"],
  ["React", "MIT"],
  ["shadcn/ui and Radix UI", "MIT"],
  ["Tailwind CSS", "MIT"],
  ["Lucide icons", "ISC"],
  ["Sonner", "MIT"],
  ["Geist and Noto Sans Hebrew fonts", "SIL Open Font License 1.1"],
];

/** About: the version, and the open-source credits with their licenses. */
export function About({ appVersion, model }: { appVersion?: string; model: string | null }) {
  return (
    <div className="flex max-h-[65vh] flex-col gap-3 overflow-y-auto">
      {(appVersion || model) && (
        <p className="px-1 text-sm text-muted-foreground">
          {appVersion && <>Version {appVersion}</>}
          {appVersion && model && " · "}
          {model && <>Speech model {model}</>}
        </p>
      )}
      <div className="divide-y rounded-xl border">
        {CREDITS.map(([name, license]) => (
          <div key={name} className="flex items-center justify-between gap-4 px-4 py-2.5 text-sm">
            <span>{name}</span>
            <span className="shrink-0 text-xs text-muted-foreground">{license}</span>
          </div>
        ))}
      </div>
      <p className="px-1 text-xs leading-relaxed text-muted-foreground">
        Free to use, share and change, never to sell; anything made from it stays under the same license.
      </p>
    </div>
  );
}
