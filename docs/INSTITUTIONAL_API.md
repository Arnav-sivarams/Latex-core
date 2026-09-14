# Institutional read API v1

The institutional API is a server-to-server, allowlisted projection of LaTeX Core business data. Its base path is `/api/integration/v1/` on the existing Caddy/application origin; it does not expose PostgreSQL or add a port. Production callers must use HTTPS. Plain HTTP is permitted only for trusted loopback development such as `http://127.0.0.1:9000`.

Every request uses `Authorization: Bearer <secret>`. Credentials in query strings are rejected because they are not authentication. A service credential is not a human session and cannot call Writer, Mentor, Admin-mutation, or collaboration WebSocket routes.

## Client administration

An authenticated Admin session and the normal CSRF header manage clients:

| Method | Path | Result |
| --- | --- | --- |
| `GET` | `/api/admin/integration/v1/clients` | Nonsecret client metadata |
| `POST` | `/api/admin/integration/v1/clients` | Create client; returns `secret` once |
| `POST` | `/api/admin/integration/v1/clients/{id}/rotate` | Replace verifier; returns the new secret once |
| `POST` | `/api/admin/integration/v1/clients/{id}/revoke` | Revoke immediately; `204` |

Creation accepts `name`, unique `scopes`, `institution_wide`, `report_ids`, and optional RFC 3339 `expires_at`. A non-institution-wide client needs at least one real report ID. The generated `lcint_` credential contains 256 random bits; only its SHA-256 verifier and display prefix are stored. Rotation invalidates the previous secret immediately. Expiry and revocation are checked from PostgreSQL on each request and require no restart.

```sh
curl -sS -X POST https://latex.example.edu/api/admin/integration/v1/clients \
  -H 'Cookie: session=ADMIN_SESSION' \
  -H 'X-CSRF-Token: ADMIN_CSRF' \
  -H 'Content-Type: application/json' \
  --data '{"name":"archive-nightly","scopes":["reports.read","reports.files.read","reports.pdf.read"],"institution_wide":true,"report_ids":[],"expires_at":"2027-01-01T00:00:00Z"}'
```

Do not put the returned secret in shell history in production; inject it through a private environment or secret manager.

## Scopes and coverage

| Scope | Permits |
| --- | --- |
| `reports.read` | report catalogue/detail and current resolved Front Matter |
| `reports.files.read` | immutable versions, manifests, and permitted file/asset bytes |
| `reports.pdf.read` | an existing successful PDF by report/build identity |
| `reviews.published.read` | published review feedback only |
| `institution.directory.read` | institutional directory/relationship collections; requires `institution_wide` |
| `institution.contacts.read` | email fields in otherwise permitted report/directory responses |

Coverage is evaluated on every list, detail, nested, continuation, and binary route. `institution_wide` is an explicit single-institution grant; otherwise only `report_ids` are visible. Directory collections are never available to report-only clients. Knowing an out-of-scope report, version, file, blob, or build ID does not grant access.

## Resource catalogue

| Method | Path | Scope | Response fields |
| --- | --- | --- | --- |
| `GET` | `/reports` | `reports.read` | `id`, title, lifecycle/review status, programme/year/semester, template and pack identities, timestamps |
| `GET` | `/reports/{report_id}` | `reports.read` | report fields plus ordered Writers, Leader, assigned Mentors and optional contacts |
| `GET` | `/reports/{report_id}/front-matter` | `reports.read` | current pack/hash/revision, semantic fields, plain values, origin, source identity, status, warnings, Guide/Dean identities, typed project metadata |
| `GET` | `/reports/{report_id}/versions` | `reports.files.read` | stable version/snapshot IDs, epoch/revision/state hashes, immutable manifest and captured metadata status |
| `GET` | `/reports/{report_id}/versions/{version_id}/files` | `reports.files.read` | permitted stable file IDs/paths, SHA-256, sizes, content URLs |
| `GET` | `/reports/{report_id}/versions/{version_id}/files/{file_id}/content` | `reports.files.read` | immutable bytes with content type and SHA-256 `ETag` |
| `GET` | `/reports/{report_id}/builds` | `reports.pdf.read` | successful build/version/source identities, PDF hashes, current/stale flag and download URL |
| `GET` | `/reports/{report_id}/builds/{build_id}/pdf` | `reports.pdf.read` | existing PDF bytes, SHA-256 `ETag`, and exact version header |
| `GET` | `/reports/{report_id}/reviews/published` | `reviews.published.read` | published thread metadata/messages only |
| `GET` | `/directory/{resource}` | `institution.directory.read` | allowlisted institutional records/relationships |

Directory `resource` is one of `students`, `faculty`, `programmes`, `departments`, `schools`, `course-registrations`, `faculty-roles`, or `department-roles`. Missing display data is `null`; it is never fabricated.

Collections use deterministic ordering and `?limit=1..100&cursor=...`. Follow only `page.next_cursor`; malformed cursors return `400`. Reports also support `programme`, `academic_year`, `semester`, and `state`. Live pages are not a global point-in-time snapshot. V1 therefore recommends periodic full reconciliation with idempotent upserts and makes no lossless change-feed/deletion guarantee.

```sh
curl -sS 'https://latex.example.edu/api/integration/v1/reports?limit=50&programme=CSE' \
  -H "Authorization: Bearer $LATEX_CORE_INTEGRATION_TOKEN"
```

## Metadata and archive consistency

Current Front Matter uses the existing materialization resolver for modern `{{field_key}}` packs and legacy VIT macro packs. Each field supplies its canonical semantic `key`, human-readable `value`, `origin` (`database`, `team_override`, `pack_default`, or `unresolved`), relevant `source_identity`, and missing/resolved status. The response also identifies pack version/content and report revision.

`project_metadata` keeps canonical institutional fields separate from Team-entered values. Its allowlisted projection includes project title; ordered Student names/registration numbers; Guides; every resolved department; Schools; semester; academic year; executive summary; `capstone`/`project-1` type; structured datasets; literal source-code snippets; structured publications; setup/revision timestamps; and per-field provenance. Missing institutional display names stay `null`. Code is stored literally and escaped only when materialized as TeX.

Versions expose metadata captured in their immutable manifest. Older versions lacking a structured capture return `captured_front_matter.status = "not_recorded"` and `captured_project_metadata.status = "not_recorded"`; current institutional values are never reconstructed as historical truth. A successful PDF is retrieved only by its stable report/build ID and includes its exact version identity. `is_current` in build metadata distinguishes a matching current PDF from a stale last-good PDF. A missing build returns `404 not_compiled`; no GET request compiles, changes a report, refreshes metadata, or changes review state.

## Exclusions and limits

Responses are explicit allowlists and never include passwords/verifiers, sessions, reset or API tokens, SMTP/outbox credential payloads, environment data, private keys, host paths, raw audit payloads, or unpublished Mentor drafts. Private personal reports are not included in institution-wide Team exports. Ordinary hidden-system files remain excluded; the sole exportable managed exception is `.latex-core/frontmatter/Front-Matter.tex`, exposed only in an authorized immutable version manifest.

Requests are limited to 100 rows and 120 authenticated reads per client per minute across API processes. Source/image downloads remain subject to the application's bounded blob rules; compiler artifacts retain compiler output limits. Minimal audit rows contain client ID, GET route/scope, outcome, and time—not credentials, query contents, or returned payloads. Common errors are JSON `{schema_version, code, error}` with `400 invalid_request|invalid_cursor|invalid_limit`, `401 authentication_required|invalid_credential`, `403 scope_denied|coverage_denied`, `404 not_found|not_compiled|not_available`, and `429 rate_limited`.

## Sample archive client

```sh
export LATEX_CORE_INTEGRATION_TOKEN='from-private-input'
python3 examples/institutional-archive/archive.py \
  --base-url http://127.0.0.1:9000 \
  --output /tmp/latex-core-archive
```

The demonstration client uses only HTTP, walks paginated report/version metadata, downloads permitted files and existing PDFs, verifies SHA-256 hashes, records stale/missing PDF status, and atomically replaces output files for safe reruns. It never prints or writes the token.

The normative route/shape contract is [OpenAPI 3.0](openapi/institutional-api-v1.yaml).
