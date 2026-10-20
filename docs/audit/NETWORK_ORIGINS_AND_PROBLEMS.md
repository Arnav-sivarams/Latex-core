# Network origins and compact Writer diagnostics

Baseline: `7e2a640e3c9c1d8cbe032b4662538a9b8d57a3b3` on
`feature/manual-compile-review-lock-institutional-api` (CI #30 passed).
This includes `44ce8ea` and CI #29's Clippy correction, `15ce044`.

## Origin audit

The review compatibility fix is already correct. `review.js` hashes the exact
UTF-8 context with `lib0/hash/sha256` and generates publication UUIDs with
`lib0/random`, backed by `crypto.getRandomValues`. It retains a pending submission
ID for retries. Do not replace these with secure-context-only `crypto.subtle`
calls, `crypto.randomUUID`, or `Math.random`.

Before this delivery, the normal development API container was labeled
`15ce044`, image
`sha256:567aa5cf70ff7e4b64997c3bd7755766e84346ebc26c99ff74d6e783cbe4bb9b`.
Its served Mentor bundle matched both `44ce8ea` and the baseline exactly:
SHA-256 `8536ec1bc9e782b40f8a1e132c6fb3a0c937572d3f325c6aaabf83bf218e1d39`.
Its Writer bundle also matched the baseline; its Admin bundle did not.
Thus the development installation was behind, but already contained the
cryptographic correction. This does not establish the state of staging.

| Surface inspected | Finding |
| --- | --- |
| Writer, Mentor, Admin, Home, legacy client, team chat | Browser API and asset requests use same-origin paths. |
| Writer/Mentor collaboration | Host and port come from `location.host`; HTTPS selects `wss:`, HTTP selects `ws:`. |
| Source anchor hashing and publication IDs | Existing compatible helpers preserve exact SHA-256 and secure randomness. |
| Bundled dependencies | lib0 reads the optional `crypto.subtle` property without invoking it in this flow. PDF.js tests for `randomUUID` and falls back to `getRandomValues`. |
| Copy LaTeX path | Clipboard access is caught; a copy prompt is available when clipboard access is unavailable or denied. |
| PDF downloads | Authenticated relative download URLs; Writer source export remains denied. |
| Login and session handling | Relative redirects; host-scoped root cookies retain HttpOnly, SameSite=Lax and configured Secure. |
| Origin checks/proxies | Origin must match Host. No arbitrary CORS allowlist or forwarded-host trust was added. The configured Caddy proxy preserves the request host. |
| CSP | No new CSP policy or bypass was introduced. |
| API binding | Compose binds the API on its private network; only Caddy's configured host publication exposes browser access. |
| Localhost references | PostgreSQL diagnostic publication, health checks and test listeners are intentionally local. SVG namespace URLs are identifiers, not network requests. |
| Credential email links | Intentionally use operator-configured `LATEX_CORE_PUBLIC_BASE_URL`; set it to the authorized public origin when enabling mail. |

The development LAN address observed for this audit was `172.19.88.175` on
`eth0`; the normal development proxy uses port `8080`. No supported HTTPS
origin was configured in this checkout. Broader deployments should use the
institution's trusted HTTPS reverse proxy, preserve Host and WebSocket
upgrades, and retain `SESSION_COOKIE_SECURE=true` with an HTTPS public URL.
Do not disable Secure cookies on an HTTPS installation to accommodate HTTP.

## Diagnostic presentation

`writer-problems.mjs` renders the original ordered diagnostics as native buttons.
Each row contains a 16px severity icon, an ellipsized message, and an aligned
file/line reference. The complete message, location and diagnostic code remain
in the hover title and accessible name. Keyboard activation and mouse clicks
use the existing `openLocation` function and original diagnostic object.
No compiler, parser, severity, count, filtering or ordering logic changes.

The expanded browser sequence also exposed a retained-session recovery issue:
after visiting multiple files, a Document Details save changes the document
generation. Rejoining a retained file from the previous generation must reload
even while the metadata save handler is finishing. The Writer now does so,
preserving the old text for recovery and avoiding a permanently conflicted
session that blocks Compile. Ordinary file switching still retains undo/redo.

## Reproduce acceptance

Install the locked npm dependencies and Playwright browser dependencies, and
make the frozen M7 image available. Build the committed frontend assets before
running tests:

```sh
npm ci
npm run build
npm test
./scripts/test-origin-browser.sh
```

The runner creates its own temporary PostgreSQL container using a tmpfs, binds
its diagnostic port only to loopback, creates a fresh synthetic database for
each origin, and removes only that container afterward. It never reads the
deployment `.env` or uses development/staging data. Both origins execute the
same Rust application and compare served asset bytes with the source files.
`network` resolves a real, non-loopback IPv4 interface, without browser security
flags.

The browser suite performs real logins, durable source edits, collaboration,
per-file undo/redo, Document Details saves, frozen-compiler builds, PDF
downloads, review locking, source selection/right-click comments, private
source/PDF drafts, publication and idempotent retry, Writer feedback navigation,
policy changes and grouped credential handoff. It rejects runtime exceptions,
server errors and unexpected failed requests.

Diagnostic fixtures intercept only the intelligence response for the layout
subtest; compilation and review acceptance use the actual backend and compiler.
The layout subtest covers zero, one and many diagnostics, long Unicode text,
missing locations, ordering/counts, native keyboard activation, source
navigation, both themes, and 375/768/1440px viewports. To retain screenshots,
set `LATEX_CORE_ORIGIN_EVIDENCE` to a new temporary output directory.

For additional byte and rendered-PDF template fidelity checks, set
`LATEX_CORE_FIDELITY_OUTPUT` to a new temporary directory and
`LATEX_CORE_PRESERVATION_TEXLIVE_IMAGE` to the frozen M7 image above when running
the browser suite. These checks only operate on imported synthetic copies.

Additional database-test qualification: the broad opt-in server database run
reported 24 passes and three failures. The import-count test passed when rerun
on its required fresh database. The standalone
`s7_bounded_concurrency_smoke_12_websocket_clients` and
`s7_governed_team_and_personal_restoration_is_append_only_and_epoch_safe` tests
also fail on clean databases at the unmodified baseline `7e2a640`: their Team
setup assumes a default template without installing one, then unwraps a missing
Team ID. In the combined run they instead inherit earlier tests' template
records whose temporary blob stores have gone away. These existing fixture
issues are separate from the successful browser collaboration, review and
policy regressions and are not changed in this delivery.

## Deployment handoff

Use the exact delivered commit from the final handoff, never a moving branch
tip without checking it. Follow [the application-only update workflow](../UPDATE_SERVER.md).
This change has no migrations and only requires updating `api`; the Worker and
frozen compiler are unchanged. Keep the existing `.env`, ports, Compose project,
database and blob volumes.

After the update, compare `/static/writer.js`, `/static/review.js`,
`/static/shells.css` and `/static/admin.js` with the pinned checkout's files.
Record the running API image ID and verify readiness and the actual browser
review workflow on the administrator's authorized LAN/HTTPS origin. Source
tests or development success alone cannot verify staging.
