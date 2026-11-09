# Live comments

Signed-in users comment on a broadcast page. Everyone on the page sees new, edited and deleted comments in real time over SSE. Authors edit or delete only their own comments. The owner can turn on slow mode.

## Sub-features
- REST CRUD. Editing or deleting someone else's comment gives 403 `NOT_COMMENT_AUTHOR`, and a stale `If-Match` gives 409.
- SSE stream with `Last-Event-ID` resume. Updates are batched about every 200 ms.
- Limits:
  - Comments are at most 500 characters. 501 gives 400, while 500 Korean characters give 201.
  - A user can post once per second. The second POST gives 429 with `Retry-After`.
  - A client can hold at most 20 streams per IP. The 21st gives 429 `TOO_MANY_STREAMS`.
- Slow mode applies to viewers. The owner is exempt.
- Comment bodies render as text only. `<script>` shows as literal text and never runs.

## How to get to it (user POV)
1. Log in.
2. Open `/broadcasts/<id>` and scroll to `section.comments[aria-label="댓글"]`.
3. Type in the `댓글` input, which shows a `n/500` counter, and press `등록`.

## Driving it with Playwright
`bash scripts/run.sh comments` ([journeys/comments.mjs](../scripts/journeys/comments.mjs)):
1. The author (role `author`) creates a READY broadcast. A second signed-in user (role `reader`) opens the same page.
2. The author posts `<script>alert(1)</script> <run id>` with `등록`.
   - The reader shows it within 3 s with no reload.
   - Neither page gains a `<script>` element.
3. After 1.1 s the author posts twice in a row. The second gets the form alert `…초 후 다시 쓸 수 있습니다.`
4. The reader has no `수정` button on the author's comment. The author edits through `form.comment-edit` (`댓글 수정`, `저장`), and the reader shows the new text with `(수정됨)`.
5. The reader calls `PUT /api/broadcasts/<id>/comments/<commentId>` with `If-Match: "<version>"`. Expect 403 `NOT_COMMENT_AUTHOR`; without `If-Match` the API answers 400 `IF_MATCH_REQUIRED` before checking the author. A reader `DELETE` also gets 403.
6. The author deletes, accepting the confirm dialog. The comment disappears from the reader.
7. Resume: the reader closes its page, the author posts, and a reopened reader page shows that comment.
8. The journey deletes its READY broadcast at the end.

End state that proves it: the same comment visible in both contexts without a reload, the 429 message, the 403s, no script execution, and no CSP violations.

## Gotchas
- Comment writes are serialized per broadcast with an advisory lock (#11 F1), and the write throughput ceiling is unmeasured. Do not fire bursts from one journey and call a slow result a bug without measuring.
- SSE goes through Caddy with compression on. Events still arrive in about 0.4 s. A journey that waits more than a few seconds for an event is seeing a real failure.
