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

Provenance methods are `MODE`, `TIE_FIRST_WRITER`, `GLOBAL_FALLBACK`, and `MANUAL_OVERRIDE`. The diagnostic preview API still returns counts, the chosen programme, tie-break information, template ID, and warnings. Normal administration shows only contextual language such as “Automatically selected from CSE” or “Selected using Writer-order tie-break”; there is no permanent resolution-calculator panel.

## Immutability and overrides

Materialization clones the selected template into one V2 workspace and writes the existing immutable `paper_template_pins` record. `paper_template_resolutions` records why it was selected. Changing a programme default affects only later resolutions; it never rewrites an existing Team pin or resolution.

An explicit template on manual Team creation is recorded as `MANUAL_OVERRIDE` and always wins. The Admin UI also supports a separate existing-Team preview/apply workflow. It never propagates programme-default changes to pinned Teams, never performs a line merge, never deletes a Writer file, and blocks when current content differs from the old template unless the file is explicitly `TEMPLATE_MANAGED`.

The global fallback and every programme default must point to an existing immutable template whose configured Main path exists in its template files. Both the Template Library and Automatic Defaults are managed from the single **Templates** page. The UI always states that mapping changes affect future Teams only.

## Admin endpoints

- `GET /api/admin/v2/institution/template-defaults/programmes`
- `PUT /api/admin/v2/institution/template-defaults/programmes/{programme_code}`
- `DELETE /api/admin/v2/institution/template-defaults/programmes/{programme_code}`
- `POST /api/admin/v2/institution/template-defaults/resolve-preview`
- `GET|PUT /api/admin/v2/institution/template-defaults/global-fallback`

All endpoints require the canonical V2 Admin global role. Institutional Admin rows alone provide no access.

Existing-Team override endpoints are:

- `POST /api/admin/v2/paper-teams/{id}/template-change/preview`
- `POST /api/admin/v2/paper-teams/{id}/template-change/apply`

Apply requires the preview token and any Main-file confirmation. It retains a pre-change safety checkpoint and a post-change template-update version.
