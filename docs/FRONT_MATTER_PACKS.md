# Front Matter Packs

Front Matter Packs are immutable, reusable application artifacts for the pages before a paper's ordinary body. They are separate from Main Content Templates and live entirely in `latex_core`; VCAP remains the institutional source for people, roles, programmes, departments, and schools.

## ZIP layout

An Admin imports one bounded ZIP containing `frontmatter.json`, one or more `.tex` files, and optional safe PNG, JPEG, PDF, or BibTeX assets. The shared template archive reader rejects traversal, absolute paths, symlinks, duplicate normalized paths, malformed archives, dangerous entry types, too many files, and oversized input or expansion.

```text
frontmatter.json
frontmatter.tex
cover.tex
certificate.tex
assets/logo.png
```

Pack source and assets are immutable and BlobStore-backed. Name and description are editable. A pack in use as a Team pin or default cannot be removed.

Pack TeX should reference sibling files through their fixed managed paths, for example `\input{.latex-core/frontmatter/cover.tex}` and `\includegraphics{.latex-core/frontmatter/assets/logo.png}`. This keeps resolution deterministic from the paper workspace root.

## `frontmatter.json`

Schema version 1 has an `entry_file`, ordered `sections`, and pack-specific `fields`:

```json
{
  "schema_version": 1,
  "entry_file": "frontmatter.tex",
  "sections": [
    {
      "key": "cover",
      "label": "Cover page",
      "file": "cover.tex",
      "required": true,
      "default_enabled": true
    }
  ],
  "fields": [
    {
      "key": "paper_title",
      "label": "Paper title",
      "type": "TEXT",
      "required": true,
      "source": "team.name",
      "default": null,
      "allow_team_override": true
    }
  ]
}
```

Section and field keys are unique lowercase identifiers. Entry and section files must exist and be TeX files. Required sections are always enabled. Supported field types are `TEXT`, `MULTILINE`, `DATE`, and `BOOLEAN`; values must match their declared type.

## Automatic sources

Manifests can use only this allow-list:

- `team.name`, `team.academic_year`, `team.semester`, `team.dominant_programme_code`
- `writers.names`, `writers.registration_numbers`, `writers.names_and_registration_numbers`
- `leader.name`, `leader.registration_number`
- `mentor.name`, `mentor.honorific`, `mentor.designation`, `mentor.faculty_id`
- `department.id`, `school.id`

The VCAP schema has no human-readable Department or School names. `institution_name`, `campus_name`, `department_display_name`, and `school_display_name` must therefore be pack defaults or permitted Team overrides. Manifests cannot contain SQL or arbitrary source expressions.

## Placeholders and safety

TeX files support only `{{field_key}}`. There are no expressions, conditions, loops, includes derived from values, or executable template language. Every placeholder must name a declared field at import time.

All automatic, default, and Team-entered values are rendered as LaTeX text. Backslashes, braces, `$`, `&`, `#`, `%`, `_`, `^`, and `~` are escaped. Arrays such as Writer lists and multiline fields have dedicated line/paragraph rendering. Static TeX in the Admin-imported pack remains trusted ordinary TeX.

## Main-template integration

A compatible immutable Main Content Template contains this exact line at its intended insertion point:

```tex
\input{.latex-core/frontmatter/frontmatter.tex} % LATEX_CORE_FRONT_MATTER
```

Template import and listing report either “Front Matter compatible” or “Front Matter not enabled.” Assignment is blocked with “This template is not configured for Front Matter.” when the marker is absent. LaTeX Core never searches for or rewrites a guessed insertion point.

## Defaults, assignment, and document details

The Templates page manages Main Content Templates, Front Matter Packs, and one Automatic Defaults table. Each programme independently chooses a Main Content Template and a Front Matter Pack. The global Front Matter fallback is nullable; `None` is valid. Changes affect future Teams only and never mutate existing pins.

Imported and manually created Teams reuse the already-resolved dominant programme. Compatible defaults are pinned and rendered; incompatible optional Front Matter records `FRONT_MATTER_TEMPLATE_INCOMPATIBLE` without blocking Team creation. Admins can independently change or remove a Team pack from Team management.

The Writer Team Leader uses the compact **Document details** action to enable optional sections and save permitted fields. Regular Team Writers can view the same details but cannot edit them. Mentors cannot edit Front Matter. Institution-assigned packs cannot be removed by Writers.

## Managed files, history, and rebuilds

Rendered files live under `.latex-core/frontmatter/` with `HIDDEN_SYSTEM` policy. They are compiler-visible but absent from participant file lists and Yjs rooms; Writer, Mentor, structural undo, rename, and delete APIs cannot reach them.

Assignment and save flush collaboration, resolve automatic/default/override values, validate required fields, render and BlobStore all outputs, then replace the managed subtree in one locked workspace event. Failure preserves the previous render. The workspace revision changes, so the normal exact-state build path includes Front Matter in compile hashes and schedules the next build from the Writer client.

Paper version manifests include both rendered managed files and Front Matter pin/value/section state. Team safe-revert restores the matching source, metadata, rendered files, and policies while retaining later history. Admin change/removal creates a `front_matter_update` safety version; removal leaves a safe empty managed entry file.

Institutional identity edits enqueue affected pinned Teams in the bounded PostgreSQL rerender queue. No Redis or external broker is used.
