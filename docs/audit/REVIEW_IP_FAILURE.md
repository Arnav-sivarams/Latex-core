# Mentor review on HTTP IP origins

Baseline: `c71fae8992e65d81190b583ae887cb711dd5d9d3`, on
`feature/manual-compile-review-lock-institutional-api`. This preserves CI #31 and
all changes at that branch tip. No ports, firewall rules, authorization policies,
institutional templates, database contents or compiler behavior were changed.

## Findings and boundaries

The branch already contained compatible SHA-256 (`lib0/hash/sha256`) and secure
UUID (`lib0/random`, backed by `crypto.getRandomValues`) implementations. Its
review asset hash before this change was
`8536ec1bc9e782b40f8a1e132c6fb3a0c937572d3f325c6aaabf83bf218e1d39`.

The reachable existing application at `http://127.0.0.1:9000` served different
assets during inspection:

| Asset | SHA-256 |
| --- | --- |
| review.js | `4c17bbbe2e04797ae6756915e289e92294627b32d0d3d32155178c1ec4f90da7` |
| writer.js | `3f34326651cc448dbb16a2c4fa17f462a4b5ddb41558251c012b990db3104d40` |

The review asset exactly matches historical commit
`38f7fcf7c08ef7cd57fe3b14d04e8de7c16117cf`. It contains secure-context-only
`crypto.subtle` and `crypto.randomUUID` calls. Those APIs are absent on ordinary
non-loopback HTTP origins: the former interrupts source-anchor creation and the
latter interrupts publication before its POST. A private draft save does not
establish publication. The existing compatible branch must actually be deployed
and loaded before this historical defect is resolved for that browser.

A separate source defect retained that historical code in caches: the Mentor
bundle used the unchanged `/static/review.js?v=v2-run2` URL with
`Cache-Control: public, max-age=31536000, immutable`. Each origin has its own
browser cache. Changing the server alone cannot invalidate a still-fresh cached
response under that URL. This release uses a new Mentor asset URL and requires
revalidation for the mutable Writer/Mentor asset paths.

Read-only inspection located port 9000 in the `LatexCore-UAT-2204` WSL
distribution, Compose project `latex-core-fb2c94395ce9`, source snapshot
`/home/professor/latex-core` (no `.git`), API image
`sha256:cfdda6696328867d4bb5f299e8cd4d0fa52bd4b109a3c826376df931814b41d4`.
A second UAT snapshot also exists; both are preserved. These observations do not
identify either installation as the user's staging deployment. The authorized
staging URL, SSH target and Git checkout were unavailable; staging source,
running image, served/cached assets, and workflow remain **UNVERIFIED**.

## Implementation

The source context-menu handler captures the range and editor state, prevents
the native menu synchronously for eligible selections, then resolves the anchor
and opens the popup. It checks that the editor/document and pending anchor still
match. Failure clears the composer and gives an actionable visible error.
Optional SyncTeX projection does not block source annotation. Normal menus
outside valid review selections and Mentor read-only permissions are retained.
SHA-256 inputs and secure identity semantics are unchanged.

A publication retry previously refreshed rounds and refused to send when the
participation was no longer pending. A successful commit followed by a lost
HTTP response could therefore strand the retained submission ID. The UI now
retains the exact paper, round, revision and submission ID, permits an explicit
retry even after the round closes, and accepts the idempotent authoritative
response. HTTP 4xx rejections allow a corrected request; uncertain transport and
5xx failures retain the identity. Publication is distinct from saving private
drafts. Only the server response or a server-confirmed submitted participation
can produce the published status. Refresh-list errors after publication do not
misreport a failed publication.

Server success/rejection responses produce `REVIEW_PUBLICATION_RESPONSE` runtime
records with server request ID, actor when authenticated, paper/round/submission
IDs, outcome, HTTP status and bounded error code. Existing transactional
`review.feedback.published` audit events remain authoritative and idempotent.
Browser-only failures cannot create publication audit events.

An assigned authenticated Mentor can send code-only diagnostics to
`POST /api/v2/reviews/papers/{paperId}/client-errors`. Schema 1 allows only
`SOURCE_ANCHOR_FAILED` or `PUBLICATION_UNCONFIRMED` and an optional round UUID;
unknown fields and arbitrary text are rejected. Reports use exact-origin CSRF
checks and a bounded process-local limit of five reports per actor per minute
(maximum 1,024 actor windows). Logs explicitly say `CLIENT_REPORTED_ERROR` and
`verified_publication=false`; the supplied round is client-reported context.
No comment text, source selection, credentials, tokens or cookies are accepted
by the diagnostic schema or included in these logs. If reporting itself fails,
the visible error remains and Console warns that reporting was unavailable.

Admin Runtime Logs already receives the selected Compose project through
`LATEX_CORE_COMPOSE_PROJECT`; it checks both container output streams and exposes
inspection errors. The deployment verifier compares this effective setting
against the actual running container label. It does not assume that the
operator's deployment is named `latex-core`.

HTTP requests remain same-origin relative paths. Collaboration uses
`location.host` and `ws:`/`wss:` according to the page scheme. Exact-origin CSRF,
Host preservation through Caddy, HttpOnly/SameSite cookies and configured Secure
cookies remain unchanged. No CORS expansion or insecure randomness is added.

## Reproduce and verify

Build assets first, then run the disposable database/real compiler/browser suite:

```sh
npm run build
npm test
./scripts/test-origin-browser.sh
```

The suite defaults to port 9000 and runs Chromium on localhost, 127.0.0.1 and a
real non-loopback IPv4 interface, followed by Firefox on that IP. It compares
served and browser-loaded bundle hashes, records origin/secure-context
capabilities, and uses actual editor drags, right clicks and Push Review clicks.
It covers Leader save/compile/send, file switching, source integrity, visible
anchor failure, private source/PDF drafts, refresh persistence, publication after
a simulated lost response, exact-request button retry and Writer source
highlights/replies. The normal extended run also checks collaboration, policy
changes, subsequent real builds and credential handoff.

If another application owns port 9000, leave it intact and qualify the port used:

```sh
REVIEW_PORT=19000 ./scripts/test-origin-browser.sh
# Test the actual non-loopback interface on port 9000 independently:
REVIEW_BIND_ADDRESS=172.19.88.175 REVIEW_PORT=9000 \
  REVIEW_TARGETS='chromium:network firefox:network' \
  ./scripts/test-origin-browser.sh
```

Replace the bind address with the host's actual IPv4 address. These are test
listener options; they do not alter deployment binding or public-base settings.
Loopback acceptance on 19000 does not verify loopback port 9000's other app.

On the authorized staging host, use the existing deployment checkout and its
existing environment/project. Pin `APPROVED_SHA` to the delivered full commit:

```sh
git fetch origin feature/manual-compile-review-lock-institutional-api
test -z "$(git status --porcelain)"
git switch feature/manual-compile-review-lock-institutional-api
git merge --ff-only "$APPROVED_SHA"
test "$(git rev-parse HEAD)" = "$APPROVED_SHA"
./scripts/update-deployment.sh api
./scripts/verify-review-deployment.sh "$APPROVED_SHA" "$STAGING_ORIGIN"
```

This release has no migrations. Keep the existing database, blob volumes,
institutional templates, `.env`, compiler image and Compose project. The update
helper validates the migration ledger before replacing the API.

Before clearing the affected browser's cache, record its actual `location.origin`,
`isSecureContext`, `typeof crypto.randomUUID`, `!!crypto.subtle`, Console exception,
and Network response bytes for review.js/writer.js. Hash those saved asset files
and compare them with the checkout and verifier output. Record the real Push
Review POST URL/status/body, GET rounds, Mentor private threads and Writer
published threads; a toast is not evidence. Then reload the changed asset URL
and repeat the full workflow using a newly created disposable report through
that browser's UI. Confirm source and PDF feedback/highlights and inspect Admin
Audit plus Runtime Logs on the actual Compose project. Never clear or overwrite
real documents to perform acceptance.

## Completed validation and remaining delivery gates

- Frontend build completed; all 128 frontend tests passed.
- `cargo test --workspace`: 193 tests passed, zero failures.
- `cargo clippy --workspace --all-targets -- -D warnings`, Rust formatting,
  whitespace checks and shell/embedded-Python syntax checks passed.
- The focused PostgreSQL review authorization/lifecycle/suggestion/isolation test
  passed, including publication privacy, revision conflicts and idempotent audit.
- Deployment update failure-path, seven worker staging cases and installer
  configuration checks passed.
- Chromium completed the real core workflow at `localhost:19000` and
  `127.0.0.1:19000`.
- Chromium and Firefox completed it at `172.19.88.175:9000`, with
  `isSecureContext=false`, `crypto.subtle` absent and `crypto.randomUUID` absent.
  Both verified selection/right-click, handled anchor failure, source/PDF draft
  privacy, refresh/file switching, actual publication POSTs, HTTP 409 rejection,
  exact-request button retry after a simulated lost response, and Writer source
  navigation/highlights. Served and browser-loaded assets matched checkout.
- The earlier concurrent port-9000 run failed during fixture login (500 instead
  of 303), before browser acceptance. The resumed sequential run with
  `CARGO_BUILD_JOBS=2` passed both browsers. No application authentication change
  was made on the strength of that interrupted/concurrent run.
- The extended policy suite passed the core workflow but later timed out waiting
  for the Writer read-only editor after starting a subsequent review round.
  That failure remains recorded; the extended suite is not reported as passed.

Exact localhost/127.0.0.1 port-9000 acceptance remains unverified because the
preserved older UAT application owns that listener. The existing development
application target must be confirmed before replacing it; the repository's
configured deployment did not have an API container on the available daemon.
Staging access and staging browser cache evidence remain unavailable. The local
source commit can be reviewed independently, but the requested push gate remains
open until exact-origin acceptance is completed or the port qualification is
explicitly accepted. Push, CI for that new commit, and deployment are pending.

Changed files are the Mentor frontend, selection helper and tests, its generated
bundle, review HTML/API and code-only diagnostic module, Writer cache headers,
browser runner/asset checks, deployment verification helper and this report.
Architecture remains Axum/PostgreSQL with private drafts and separate authorized
idempotent publication. There are no migrations or new production services.

## Final Admin Audit acceptance

The final acceptance request permits an available test port on the assigned
non-loopback IP, superseding the earlier exact-loopback-port delivery gate.
The focused run used `http://172.19.88.175:19000`, disposable PostgreSQL/Team data,
real browser logins, source editing, compilation and review buttons. The older
UAT installation and existing databases/volumes were preserved.

Inspection found a narrow durable-audit gap: Leader Send for Review logged a
runtime message but no audit event. Successful new rounds now insert
`review.round.opened` inside the same transaction as round/participation creation.
Existing round retries return before insertion. Mentor publication retains its
transactional `review.feedback.published` event and retry path; success metadata
now explicitly includes the outcome. The Admin projection exposes only safe
review identifiers and outcomes, and the real Audit UI labels and displays both
operations with actor, paper/Team, round, timestamp and success outcome.

Focused acceptance command:

```sh
CARGO_BUILD_JOBS=2 REVIEW_PORT=19000 REVIEW_TARGETS=chromium:network \
  REVIEW_SKIP_POLICY=1 REVIEW_AUDIT_ONLY=1 ./scripts/test-origin-browser.sh
```

It passed the actual source/PDF annotation and publication workflow, checked
Admin Audit after Leader send, after an authorized HTTP 409 revision rejection,
and after publication/retry, and verified feedback after reload. Direct
PostgreSQL assertions found exactly two success events: one for the authorized
Leader and one for the assigned Mentor, referencing the same paper and round,
with timestamps and `SUCCESS` metadata. Rejection created no publication audit;
retry returned the original submission and created no duplicate audit record.

Admin Runtime Logs was checked separately through the actual UI and returned
HTTP 200 from the configured container source. Those logs were not substituted
for the test API process's durable audit evidence. The runner also supports
visibly unavailable runtime sources without inventing audit events.

Only focused review/audit acceptance, scoped Clippy with warnings denied,
formatting, JavaScript syntax and diff checks were run for this correction.
Previously passing broad suites were not repeated locally. CPU-heavy commands
ran sequentially with two Cargo jobs. This correction needs no migration and
introduces no service or infrastructure changes. Staging remains UNVERIFIED;
no staging deployment is authorized by the final acceptance request.

## IP configuration and rebuilt API identity (2026-10-29)

The professor's report that changing an address in `main.rs` made review work is
accepted as observed working evidence. The supplied location, line 12354 in
`1e4eab11d0d91be6495b02c462b2771146ec05d5`, is the address in the network-origin
regression fixture. Editing this compilation unit and rebuilding also rebuilds
the executable that embeds review.js, writer.js and admin.js. That edit/rebuild
sequence therefore changes the deployed artifact and can load corrections that
were absent from the previous executable or browser cache.

Read-only inspection of the actual development deployment demonstrated this
source/runtime divergence. API image
`sha256:dfeeab3052d8f7c98ce0371363b06b7fe661a296fa8911db8cd012b90824f94f`
had no source revision label. Its served review.js matched the checkout, but its
admin.js SHA-256 was
`63462cbad86d1cf684617d922bb823e933ee5a7faa90a5d7179d41d8be9e386a`, exactly
matching `6668701`, rather than the checkout's
`a506cbe0ec42e6d9af0e3074d39fd21e0e0ed7ad5ac5189b8f4c06f93ad68aa1`.
Thus Git HEAD and a matching Mentor asset alone were insufficient to establish
what backend and Admin code were running. The professor's old image and full
old/new edit were not supplied, so that remote artifact's exact history remains
unverified. The identified line and the measured local drift support the rebuild
mechanism; no runtime request path reads that address as an allowlist.

The correction centralizes origin parsing in `http_origin.rs`. Request origin
validation compares the browser Origin with the actual Host authority, validates
ports and components, normalizes host case/default ports, and rejects foreign,
null or repeated Origin headers and spoofed forwarded hosts. It introduces no
CORS allowlist or forwarded-host trust. The existing treatment of non-browser
requests without Origin is retained. Network regression cases now live beside
this helper and generate multiple LAN addresses; no address in main.rs needs to
be changed to admit a new installation.

Canonical public links use the existing `LATEX_CORE_PUBLIC_BASE_URL`, validated
as a complete HTTP(S) origin. API startup, mail configuration and installation
validation reject credentials, paths, queries, fragments and invalid ports.
Mail links use this canonical URL. Review HTTP requests remain relative and
WebSockets use the current browser host and scheme, independently of the
canonical link host. Cookie roles, review authorization, private draft visibility
and transactional audit/publication remain unchanged.

The API exposes a versioned `/api/build-info` response with its compiled source
revision and SHA-256 identities of embedded review.js, writer.js and admin.js.
The deployment helper requires committed source and supplies that revision to
the image build; the image records the same revision in its OCI label. The
read-only verifier compares the running API revision, embedded hashes and served
bytes with the approved checkout, including Admin assets. Unknown/stale images
fail verification instead of being inferred correct from Git HEAD. Build jobs
are bounded at two.

The focused browser fixture can execute an API binary exported from an actual
image while using disposable PostgreSQL and BlobStore paths. This permits real
IP editor/review/audit acceptance against that executable's exact bytes without
changing real reports, accounts or templates. Its canonical link URL deliberately
uses a different hostname; same-origin actions still follow the browser request.

For a professor using plain HTTP directly on LAN port 9000, retain the existing
credentials, database, project name and storage settings, and set only the
relevant URL/listener keys (substitute the professor's actual LAN address):

```dotenv
LATEX_CORE_PUBLIC_BASE_URL=http://<PROFESSOR_LAN_IP>:9000
SESSION_COOKIE_SECURE=false
HTTP_PORT=9000
HTTP_BIND_ADDRESS=0.0.0.0
```

For production HTTPS, use the real canonical origin and Secure cookies:

```dotenv
LATEX_CORE_PUBLIC_BASE_URL=https://papers.example.edu
SESSION_COOKIE_SECURE=true
```

Keep the existing internal listener/publication port behind the HTTPS reverse
proxy. Preserve the external Host and WebSocket upgrades through that proxy.
No Rust address edit, wildcard CORS, forwarded-host allowlist or cookie security
bypass is required. Use the application-only updater and then the verifier;
reloading the browser's current asset URL must load the matching bundle.
