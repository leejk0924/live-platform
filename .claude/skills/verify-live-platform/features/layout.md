# Layout and theme

Every screen uses the design tokens from `design.md`: dark by default, light under `prefers-color-scheme: light`. The home and replay lists are card grids, and the broadcast page puts the player (or the owner's publisher) next to a donations and comments sidebar at 1100 px and wider. Phones get the same screens stacked, with no horizontal scroll and touch targets at least 44x44 px (#9, #56).

## Sub-features
- Screens: home (signed in and signed out), watch (viewer of a LIVE broadcast), studio (owner of a LIVE broadcast), studio-ready (owner of a READY broadcast), interrupted (viewer of an INTERRUPTED broadcast), an ENDED broadcast, broadcast new and edit, login, signup, recordings list and detail, `/pay/checkout`, `/pay/success`, `/pay/fail`, not found.
- Viewports: desktop 1280x800, phone 390x844 and narrow phone 360x780 (Pixel 7 emulation with the viewport narrowed). 360 px is the narrow end of the 360 to 412 px phone range.
- Color schemes: dark and light, through `page.emulateMedia({ colorScheme })`.
- Rules, each checked on every screen, viewport and scheme:
  - No horizontal scroll: `document.documentElement.scrollWidth <= innerWidth`.
  - Phone only: every visible `button`, `a[href]`, `input`, `select`, `textarea`, `[role=button]` and `summary` is at least 44 px tall and 44 px wide. A checkbox or radio is measured by its `label`. A link inside a sentence (`p` with more text than the link) is exempt, as WCAG 2.5.8 exempts inline links. A link row (`p.links`) is not a sentence, so its links are measured.
  - No browser defaults: no link with the user agent's link color, no link underlined outside a sentence, and no control whose background and border equal an unstyled one in a shadow root (which keeps page CSS out and inherits the color scheme).
  - Text contrast at least 4.5:1, or 3:1 for large text (24 px, or 18.66 px bold). The text color, scaled by the opacity of the element and its ancestors, is blended over every translucent layer down to the first opaque background. A gradient counts at its worst stop. Text under `aria-hidden`, disabled controls and `<option>` are skipped. Placeholders count.

## How to get to it (user POV)
Open each route at a desktop window and on a phone, once with the system in dark mode and once in light mode.

## Driving it with Playwright
`bash scripts/run.sh layout` ([journeys/layout.mjs](../scripts/journeys/layout.mjs)), about 7 minutes:
1. Roles `lcaster` and `lviewer` sign in. The caster goes live for 8 s and ends, which leaves a READY recording, then goes live on a second broadcast and stays live. Viewer and caster take turns posting eight comments through the API. The caster creates a third broadcast that stays READY. Role `lphone` goes live from an emulated phone (one owner cannot have two LIVE broadcasts).
2. For each viewport, a context per role (caster, viewer, guest) starts from the signed-in cookies. For each scheme, each screen is opened, waited for (feed title, playing video, publisher, form, notice), and screenshotted as `<screen>-<desktop|phone|phone360>-<dark|light>.png`. Before each interrupted screen the phone page goes hidden (`setHidden`), its publisher pauses and the broadcast turns INTERRUPTED; the screen is ready when `방송 연결이 끊겼습니다` shows. After it the phone comes back and the broadcast is LIVE again within the 30 s grace.
3. The audit runs in the page and each rule becomes one check per screen, naming the worst offenders with their size or ratio.
4. Short desktop window: the viewer opens the watch screen at 1280x600 (`watch-desktopshort-dark.png`). Scrolled to the top, the comment input (`textbox` named `댓글`) ends within the 600 px; after `scrollIntoViewIfNeeded` it lies inside the viewport; the comment list keeps at least 96 px. The check names the sidebar, donations, comments and list heights.
5. The caster and the phone press `방송 종료`, and the READY broadcast is deleted. No CSP violations on any page.

End state that proves it: `VERIFY OK (n checks)` with 103 screenshots, and the log line `failing screens per rule {"overflow":0,"touch":0,"defaults":0,"contrast":0}`.

## Gotchas
- `/pay/checkout` is opened with an out-of-range amount, so the page shows its error without loading Toss. The Toss widget itself runs in Toss's frames and is covered by the donation journey.
- `/pay/success` is opened without a payment, so it shows its error. Its success notice needs the server to confirm a real Toss payment, which only the donation journey makes; showing it here would mean faking the confirm answer.
- Contrast is computed from computed styles, not pixels. Text over the video or an image would be unmeasured; the app draws neither under text.
