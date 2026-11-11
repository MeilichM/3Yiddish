# Contributing

- **Build:** `cargo build --release` (needs a C/C++ toolchain, CMake and clang; on Linux
  also `libasound2-dev`). The speech engine (whisper.cpp) is compiled in through `whisper-rs`.
- **Check:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- **Models** are trained elsewhere and kept on Hugging Face. A candidate becomes the public
  model through the *Promote model* workflow, which scores it on a private test set and
  publishes it only if it beats the current one.
- **Releases:** bump `version` in `Cargo.toml`, tag `vX.Y.Z`, push the tag. The *Release*
  workflow builds every platform and publishes the binaries and installers.

By contributing you agree your contribution is licensed under CC BY-NC-SA 4.0.
