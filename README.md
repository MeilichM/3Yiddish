# 3Yiddish

**You talk Yiddish. It types Yiddish.** On your own computer, for free, no account,
no cloud, no nonsense. Yes, it knows loshn koydesh. Mostly. It's learning. Be nice.

## Get it (one line, we promise)

**Linux / Mac:** open a terminal, paste, hit Enter:

```sh
curl -fsSL https://github.com/TripleU613/3Yiddish/releases/latest/download/install.sh | sh
```

**Windows:** open PowerShell, paste, hit Enter:

```powershell
irm https://github.com/TripleU613/3Yiddish/releases/latest/download/install.ps1 | iex
```

That's it. It downloads itself, checks it isn't broken, and grabs the speech model
(~600 MB, one time; go make a coffee).

## Use it

```sh
3yiddish
```

Press **● Record**. Say something. Press **■ Stop**. Watch the Yiddish appear.

Got a file instead?

```sh
3yiddish transcribe shiur.mp3
```

**Type Yiddish anywhere (push-to-talk).** Run `3yiddish dictate`, then hold **F9** in any app
(email, WhatsApp, Word, a forum), speak, and let go: the Yiddish is typed right where your cursor
is. Pick another key with `3yiddish dictate --key CapsLock`. On Linux, add a keyboard shortcut in
your desktop settings; `3yiddish dictate` prints the exact line to paste. No microphone on the
PC? Pair your phone and it becomes the mic.

Fancy bonus stuff:
- **Phone as a mic.** Pair your phone from the **Phone** tab and talk into that.
- **Read-aloud game.** Read lines out loud, get a score, and help the model get smarter.
- **Local API.** `3yiddish serve`, then point any OpenAI-style transcription tool at
  `http://127.0.0.1:8793`. If that sentence meant nothing to you, ignore it.

## Updates? Automatic-ish

When there's a new version or a smarter model, the app shows a button. Click it. Done.
New models only ship after they beat the old one on our test, so they only get better.
Don't like the new one? `3yiddish model rollback`.

## Is my stuff private?

Yes. It all runs on your computer. Your voice never leaves it. The only thing it ever
asks the internet is "got any updates?"

## Rules (the short version)

[CC BY-NC-SA 4.0](LICENSE): use it, share it, change it, have fun.
**Don't sell it.** Ever. And anything you make from it stays just as free.
Credits for the giants whose shoulders we stand on: [NOTICE](NOTICE).

Want to hack on it? [CONTRIBUTING.md](CONTRIBUTING.md) has the grown-up instructions.
