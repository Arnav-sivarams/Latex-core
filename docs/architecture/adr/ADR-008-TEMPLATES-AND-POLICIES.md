# ADR-008: Templates and File Policies

## Status

Accepted for V2.

## Template versioning

A Paper Team receives a pinned immutable template version, never a live mutable template. A controlled Admin template update follows: preview, diff, policy check, checkpoint, apply, and conflict resolution if required. The resulting paper state and template-version reference are auditable and append-preserving.

## File-policy semantics

| Policy | Exact Writer semantics |
| --- | --- |
| `EDITABLE` | Content and structural mutations are permitted. |
| `CONTENT_READ_ONLY` | Read is permitted; content and structural mutations are denied. |
| `STRUCTURE_LOCKED` | Content edits are permitted; rename, move, and delete are denied. |
| `TEMPLATE_MANAGED` | Content edit, rename, move, and delete are denied; only the controlled Admin template-update workflow may alter the file. |
| `HIDDEN_SYSTEM` | The file is not normally exposed in Writer or Mentor file trees; the Admin control plane may inspect metadata. |

Set Main and create semantics are checked against the relevant paper, directory, template, main-file, and policy constraints rather than inferred solely from this table. A hidden file is not a confidentiality boundary from TeX source that can include it; artifact and source reads remain authorization-scoped.

A Mentor is always read/annotate-only regardless of the Writer policy. An Admin can change policy or apply a controlled template update but cannot use policy administration to submit an ordinary source mutation.

Policies are enforced on HTTP mutation, WebSocket source update, structural operation, and template update. UI hiding is not authorization.
