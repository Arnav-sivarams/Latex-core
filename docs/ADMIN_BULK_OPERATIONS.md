# Admin bulk operations

The Paper Team grid is a server-driven page of 25, 50, or 100 rows. Search, programme, Mentor, Leader, template, lifecycle status, review state, source, and unresolved filters execute in PostgreSQL. The browser never loads every Team to filter locally.

## Lifecycle changes

Select visible resolved Team rows and choose Freeze, Activate, or Archive. The UI requires confirmation and sends at most 200 explicit Team IDs. The server authorizes a V2 Admin, validates each Team independently, and returns one success or typed failure per Team. Partial success is reported and audited. There is no bulk delete, membership removal, template overwrite, or source overwrite.

## Imported and unresolved Teams

Imported rows show the external key, source import job, and import time. Imports are additive: omitted assignment rows do not remove existing members. The Unresolved filter reports the error code/message and unresolved institutional IDs. Resolve Identity Links or automatic defaults on the single Templates page, then return to the source import job. Idempotent import semantics and the unique external key prevent a duplicate Team. View / Manage also edits the Team name, ordered Writers, Writer Leader, and Mentors transactionally in both the institutional assignments and runtime membership without recreating the Team.

## Safe template override

Template replacement is intentionally not a bulk operation. The Team page presents a plain Change template confirmation with changed/added file counts, and shows Writer-conflicting paths when blocked. Main-document changes receive one additional confirmation. Internally, the existing exact-state preview token, collaboration flush, workspace lock, pre/post versions, and conflict validation remain mandatory; the successful change affects only the selected Team pin as `MANUAL_OVERRIDE`.

## Scale boundary

Institution directories and import history use the same page/limit contract. Full error sets are downloaded as CSV instead of rendered. These controls target the repository's 1,000 concurrent-user scope; they are not a claim of formal concurrent-user capacity.
