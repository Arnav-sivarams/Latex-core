# Programme-based Team template defaults

New Paper Teams select an immutable template from their ordered Writers unless an Admin supplies a manual template override. The institutional programme source is `vcap.students.programme_code`; course-registration `course_id` values are never used as programme identity.

## Resolution

For ordered Writer user IDs, the resolver:

1. follows a `LINKED` student identity to `vcap.students`;
2. excludes missing links or programmes and returns warnings;
3. counts each remaining programme;
4. chooses the mode;
5. on a tie, chooses the programme belonging to the earliest Writer in the supplied order;
6. uses `latex_core.programme_template_defaults` for that programme; and
7. uses the configured global fallback if the programme is unmapped or every Writer is unresolved.

Provenance methods are `MODE`, `TIE_FIRST_WRITER`, `GLOBAL_FALLBACK`, and `MANUAL_OVERRIDE`. The preview API returns counts, the chosen programme, tie-break information, template ID, and warnings.

## Immutability and overrides

Materialization clones the selected template into one V2 workspace and writes the existing immutable `paper_template_pins` record. `paper_template_resolutions` records why it was selected. Changing a programme default affects only later resolutions; it never rewrites an existing Team pin or resolution.

An explicit template on manual Team creation is recorded as `MANUAL_OVERRIDE` and always wins. Run 1 does not add a workflow for changing an existing Team's pin; that remains a separately governed Run 2 concern.

## Admin endpoints

- `GET /api/admin/v2/institution/template-defaults/programmes`
- `PUT /api/admin/v2/institution/template-defaults/programmes/{programme_code}`
- `DELETE /api/admin/v2/institution/template-defaults/programmes/{programme_code}`
- `POST /api/admin/v2/institution/template-defaults/resolve-preview`
- `GET|PUT /api/admin/v2/institution/template-defaults/global-fallback`

All endpoints require the canonical V2 Admin global role. Institutional Admin rows alone provide no access.
