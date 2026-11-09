---
name: verify-live-platform
description: Prove that live-platform works end to end by driving the real app (compose stack at https://localhost: Caddy, axum api, MediaMTX, Postgres) with Playwright on the system Chrome, the way a user would: sign up, go live, watch over LL-HLS/WHEP, comment, record, donate, publish from a phone. Use after changing crates/, web/, deploy/ or compose.yml, before claiming a feature or an issue's completion condition works, and as the reference for what "works" means per module (features/).
---

# verify-live-platform

The unit tests (`cargo test`, `npm test`) prove pieces. This skill proves the product: a real browser against the real stack, with screenshots and a pass/fail log as evidence. Each module's "works" is defined by its file in [features/](features/README.md). A change to a module is verified when its feature journey passes on a stack built from the change.

All commands run from the repo root in Git Bash. `S=.claude/skills/verify-live-platform/scripts`.

## Launch

```bash
bash $S/setup.sh     # once: playwright-core 1.56.1 into $VERIFY_HOME, checks Chrome
bash $S/launch.sh    # builds the current checkout with deploy/deploy.sh local and records what it launched
```

- Needs Docker running, `.env` filled from `.env.example` (`PUBLISH_JWT_PRIVATE_KEY`, `MEDIA_HOOK_SECRET`; `TOSS_*` only for the donation journey).
- Ready means `launch.sh` printed `ok: launched <sha>`. That line comes after `docker compose up --wait` (every healthcheck green) and `/api/healthz` answering `ok` through Caddy. The api runs migrations at startup, so an empty Postgres volume comes up usable. No seed data is needed: journeys sign up their own accounts.
- `$VERIFY_HOME` defaults to `~/.cache/verify-live-platform`: driver, launch record, lock, evidence. Nothing goes into the repo.
- To test a worktree, run `launch.sh` from that worktree. It rebuilds the one shared stack from that checkout.

## Doctor

```bash
bash $S/doctor.sh
```

Read-only. Exit 0 only when every line is `ok`. It checks four things:
- The four services are healthy, `healthz` is ok, and 443 answers with this app's CSP header.
- The stack was built from the current `HEAD`, or from a commit whose difference to `HEAD` touches no build input (`crates`, `web`, `deploy`, `docker`, `compose.yml`, `Cargo.*`). A `FAIL` here means relaunch. Proving against a stale build proves nothing.
- No other run holds the lock.
- The driver and Chrome are present.

Run it first, and again whenever anything looks off.

## Isolation

There is one stack. It owns ports 80, 443 and 8189 and a single Postgres volume, so two instances cannot run side by side, and two agents must not drive it at once. `run.sh` takes `$VERIFY_HOME/lock` and refuses to start while another run holds it. Never delete a lock whose pid is alive. Parallel worktrees take turns: launch, run, then hand over.

## Drive

```bash
bash $S/run.sh watch          # one journey: scripts/journeys/<name>.mjs
bash $S/run-all.sh            # watch, publish, comments, recording, donation, then deploy/check-hardening.sh local
bash $S/run-all.sh comments recording   # a subset (no hardening)
```

Journeys: `watch`, `publish`, `comments`, `recording`, `donation` (needs internet for the Toss sandbox). A full `run-all.sh` takes about 5 minutes. `publish` alone waits out the 30 s grace, so it takes about 1.5 minutes.

- Journeys are Playwright scripts built on [scripts/lib.mjs](scripts/lib.mjs). Its helpers are `account`, `createBroadcast`, `startPublishing`, `assertPlaying`, `waitStatus`, `broadcastStatus`, `stopBroadcast`, `phoneContext`, `setHidden`, `api` (with optional headers), `check`/`must`, `shot`, `evidence` and `noCspViolations`.
- `account(page, role)` logs in as the role's account from `$VERIFY_HOME/accounts.json`, or signs one up when there is none or login fails (a reset database). Sign-up allows 5 per client per 2 minutes, so fresh accounts per run would hit 429 in `run-all.sh`. It also stops any broadcast the account left LIVE. Each journey uses its own roles, so journeys never share a broadcast.
- The browser is the system Chrome, headless, with fake cameras, a microphone and a screen (`--use-fake-device-for-media-stream=device-count=3`, `--use-fake-ui-for-media-stream`), so publishing works without hardware. `phoneContext` hides `fake_device_1`: Chrome delivers its frames but WebRTC encodes none of them, so a viewer would freeze after a camera switch.
- CSP violations are collected per browser context across navigations, for this app's origin only. `noCspViolations` fails on any.
- Prefer these handles. They exist in `web/src`:

| What | Handle |
|---|---|
| Live player | `section.player[aria-label="시청"]`, `data-state`, `data-source="ll-hls"\|"whep"` once a source plays |
| Low-latency toggle | label `저지연(WebRTC)`. Absent on iOS |
| Publisher | `section.publisher[aria-label="송출"]`, buttons `방송 시작` / `방송 종료` / `카메라 전환`, status text `.publish-state[role=status]` (`송출 중 · <kbps> · <WxH>`) |
| Home feeds | `section.feed[aria-label="방송 중"]`, `section.feed[aria-label="지난·예정 방송"]` |
| Comments | `section.comments[aria-label="댓글"]`, input `aria-label="댓글"` (maxlength 500), button `등록`, items `.comment` |
| Donations | `section.donations[aria-label="후원"]`, `.donation-totals`, `.donation-form`, presets `5,000원`, button `후원하기`, live notice `.donation-fresh` |
| Replay | `section.player[aria-label="다시보기"]` with `data-state`, list `section.feed[aria-label="녹화 목록"]` |
| Routes | `/`, `/broadcasts/:id`, `/broadcasts/new`, `/recordings`, `/recordings/:id`, `/pay/checkout`, `/pay/success`, `/pay/fail`, `/login`, `/signup` |

- API calls from a journey go through `api(page, …)`, which runs `fetch` inside the page. The session cookie and the required `X-Requested-With: live-platform` header then apply exactly as for the real UI. Use the API only for setup (sign up, create broadcast) and for reading side effects. The behavior under test goes through the UI.

## Evidence

Each run writes `$VERIFY_HOME/evidence/<UTC time>-<journey>/`:
- `log.txt`: the full log, including every console error, every HTTP ≥ 400 and every CSP violation the browser saw.
- `checks.tsv`: one `PASS`/`FAIL` row per assertion.
- `*.png`: full-page screenshots at each step.
- `meta.txt`: the commit, the launch record and the exit code.

A run passes when it ends with `VERIFY OK` and exits 0. Quote that line and the evidence path when you claim something works.

Proof standards:
- Drive the real user path. Click the buttons, use the feeds, play the `<video>`. Do not call internal setters or test-only endpoints. There are none, so do not add them.
- Capture the action and the resulting state. Example: the broadcast `status` from the API turns `LIVE` or `ENDED`, and `currentTime` advances on the `<video>`. A screenshot of a final screen alone is not proof.
- Verify side effects alongside the visible result. Examples: the recording row turns `READY` and the file appears in the `recording_store` volume, the donation total changes in a second tab, the audit line is written to `docker compose logs api`.
- Mock only at boundaries production already isolates. Toss runs in its own sandbox with test keys (`test_gck_`/`test_gsk_`). S3 and CloudFront are not reachable locally, so use the local recording storage.
- Known noise in `log.txt`, which does not mean failure:
  - `401 GET /api/auth/me` for anonymous pages.
  - `404` on `index.m3u8?cookieCheck=1` and on the `*_stream.m3u8` blocking reloads right after going live or ending.
  - `404 DELETE …/whip|whep/<session>` after the server already kicked the session.

  Anything else at ≥ 400 deserves a look.

## Cleanup

```bash
bash $S/cleanup.sh
```

- Removes a stale lock (dead pid only) and kills Chrome processes that carry a Playwright profile, which are left only by a crashed journey.
- Stops the compose stack only if `launch.sh` found it down and started it. Volumes are kept. If the stack was already up, it stays up: it is the user's dev stack.
- Never touches `$VERIFY_HOME/evidence`.
- DB rows created by journeys stay: accounts `v<role><hex>` (reused across runs) and broadcasts titled `verify <feature> <run id>`, with description `verify-live-platform <run id>`. They are ENDED, or deleted by the journey (comments, recording). Do not delete rows from the shared DB by hand.
- `$VERIFY_HOME/accounts.json` holds the journey accounts' local passwords. They are throwaway accounts on the local stack only.

## Helpers

| Script | What it does |
|---|---|
| `scripts/setup.sh` | Install the driver, check Chrome (`CHROME=<path>` overrides) |
| `scripts/launch.sh` | Build and start the stack from this checkout, write `$VERIFY_HOME/state` |
| `scripts/doctor.sh` | Read-only health, build-matches-HEAD, lock check |
| `scripts/run.sh <journey>` | Lock, run `scripts/journeys/<journey>.mjs`, keep evidence |
| `scripts/run-all.sh [journey ...]` | Every journey in turn plus the hardening check, `ALL OK` or `FAILED: <names>` |
| `scripts/cleanup.sh` | Tear down only what this skill started |
| `scripts/lib.mjs` | Shared Playwright helpers for journeys |

A new feature gets a feature file and a journey in `scripts/journeys/` following the existing ones: end with `noCspViolations` and `summary()`, add it to `run-all.sh`'s default list and to `features/README.md`.
