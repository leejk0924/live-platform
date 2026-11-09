# Feature map

One file per user-facing feature. Each file says what "works" means for that module: how a user reaches it, how to drive it, and the observable end state that proves it. A change to a module is verified when that file's journey passes on a stack built from the change (`scripts/run.sh <journey>`). A proof that drives one convenient entry point is incomplete when the file lists others.

| Feature | Module (code) | Journey |
|---|---|---|
| [watch](watch.md) | playback (#8): `PlaybackResolver`, `web/src/broadcasts/watch/` | `run.sh watch` |
| [publish](publish.md) | publish (#7, #9): WHIP, `web/src/broadcasts/publish/` | `run.sh publish` |
| [comments](comments.md) | comments (#11): REST + SSE, `web/src/comments/` | `run.sh comments` |
| [recording](recording.md) | recording (#12): hook, storage, `web/src/recordings/` | `run.sh recording` |
| [donation](donation.md) | donation (#13): Toss sandbox, `web/src/donations/`, `/pay/*` | `run.sh donation` |
| [hardening](hardening.md) | security (#14): Caddy headers, CSP, timeouts | `bash deploy/check-hardening.sh local` |

`bash scripts/run-all.sh` runs all of them. A journey passes when it ends with `VERIFY OK (n checks)`; n is one higher on a run that had to sign up an account. Each file's Gotchas list what the journey cannot prove locally (real phones, real AWS, card payments).
