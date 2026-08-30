# ADR-006: Source and PDF Review Anchors

## Status

Accepted for V2.

## Decision

The review layer uses PDF.js with an annotation overlay, SyncTeX for PDF/source mapping, and CRDT-relative source anchors.

A source anchor contains stable `file_id`, `relative_start`, `relative_end`, quoted text, context hash, and `paper_seq_at_creation`. A PDF anchor contains immutable `artifact_id`, page, normalized rectangles, mapping confidence, and SyncTeX source metadata.

Mapping states are `exact`, `approximate`, `PDF-only`, `source changed`, and `source deleted / re-anchor required`.

## Flows

Source-first review is:

```text
Mentor source selection -> source relative anchor -> SyncTeX forward mapping -> PDF overlay
```

PDF-first review is:

```text
Mentor PDF rectangle -> persistent artifact/page geometry -> SyncTeX inverse mapping
                     -> source anchor where possible
```

After a new compile, the system reprojects from the current source anchor, retains the historical artifact anchor, and marks drift when source changed materially.

## Accuracy boundary

The product must not promise exact mapping for macro-generated content, bibliography-generated output, some complex figures and tables, or other generated TeX constructs. PDF-only feedback remains valid against its artifact even when a source anchor cannot be derived.
