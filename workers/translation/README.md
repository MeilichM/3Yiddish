# Native translation worker

Rust/Axum HTTP worker with native CTranslate2 CPU int8 inference. It loads only
explicit local model files. No Python runtime, model download, cloud inference or
web UI is used by this service. Build tools: Rust 1.98+, CMake and a C++17 compiler.
The coordinator and its Docker image remain independent of these native build
requirements.

```sh
cargo build --release --locked --manifest-path workers/translation/Cargo.toml
workers/translation/target/release/3yiddish-translate \
  --model /your/cache/nllb600m-ct2-int8 --threads 4
```

The model directory must contain CTranslate2 `model.bin`, `config.json`,
`shared_vocabulary.json` and the original `sentencepiece.bpe.model`. It is produced
once with the external CTranslate2 conversion tool:

```sh
ct2-transformers-converter --model /your/pinned/nllb600m-cache \
  --output_dir /your/cache/nllb600m-ct2-int8 --quantization int8 \
  --copy_files sentencepiece.bpe.model tokenizer_config.json special_tokens_map.json
```

That conversion tool uses Python; it is not an application service dependency.
The source HF revision is `f8d333a098d19b4fd9a8b18f94170487ad3f821d` in
`facebook/nllb-200-distilled-600M`. The model is **CC-BY-NC-4.0**, so this is a
noncommercial local research baseline, not an unrestricted 3Yiddish model release.
The executable code has the repository's current source license; native libraries
retain their own licenses. No checkpoint or third-party source is vendored here.

Add this table to the coordinator's private configuration:

```toml
[translation]
url = "http://127.0.0.1:8799/v1/translate"
model = "nllb600m-int8-local"
```

The reasoning backend remains under `[reasoners.local]`. With no `[translation]`
table, the previous OpenAI-compatible Schmooze translator remains available.

`POST /v1/translate` takes `{"text":"I can help you.","direction":"en-yi"}`.
Directions are `en-yi` and `yi-en`; NLLB tags are `eng_Latn` and `ydd_Hebr`.
`GET /healthz` reports ready only after model/tokenizer loading. The listener is
loopback-only. Requests are bounded, only one inference runs at a time, and input
beyond 512 source tokens or incomplete output is rejected rather than silently
truncated. Long messages must be split explicitly.
