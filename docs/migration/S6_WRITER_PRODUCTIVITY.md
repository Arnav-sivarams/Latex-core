# S6 Writer Productivity

S6 adds bounded Writer productivity features without changing the compiler, queue, collaboration protocol, review authorization, or immutable version model.

## Structural undo

`reversible_structural_operations` records Writer-owned create, rename/move, delete, and Set Main operations in the same PostgreSQL transaction as their canonical workspace event. Records contain stable file identity, paths, BlobStore references needed to reverse a tombstone, workspace versions, and state; they do not duplicate source text.

`POST /api/v2/papers/{paper_id}/structural-undo` reverses the requesting Writer's latest applied operation. `structural-redo` reapplies the latest actual undo. Both require current Writer access and an active paper, flush collaboration rooms, verify current file identity/path/revision or current Main, and return `409 Conflict` rather than overwrite a later structural or content change. A new structural operation invalidates outstanding redo records. Tombstones retain BlobStore and collaboration history.

## Intelligence and navigation

`GET /api/v2/papers/{paper_id}/intelligence` flushes active collaboration state and analyzes the current canonical multi-file workspace with the existing Tree-sitter LaTeX/BibTeX project analyzer. The versioned response normalizes outline sections, labels, reference/citation resolution, bibliography entries, environments, packages, and existing analyzer diagnostics against stable file UUIDs and source ranges.

Writer renders a compact outline and Problems panel. Entries open the stable file and source location. Intelligence refresh is debounced after durable collaboration acknowledgements, never on each keystroke. Canonical project search is plain-text, bounded to 200 results, and grouped in the UI by path/location. Project-wide replacement is deferred; CodeMirror's current-file search/replace remains available.

Quick Open (`Ctrl/Cmd+P`) uses local fuzzy path/basename ranking. The command palette (`Ctrl/Cmd+K`) delegates to existing actions. CodeMirror completion covers common commands/environments and provides citation keys and label references from intelligence. Symbols and snippets insert through Writer-local Yjs transactions.

Problems combines analyzer diagnostics (including unresolved references/citations) with the latest build failure. Writer source-to-PDF uses the existing S5 SyncTeX mapping and current PDF iframe page navigation. Browser-native iframe coordinates do not provide a dependable inverse mapping, so Writer PDF-to-source clicking is deferred; Mentor PDF.js inverse navigation is unchanged.

## Assets and builders

Writer asset upload accepts validated PNG, JPEG, PDF, and CSV files up to the server's 1 MiB file limit, stores bytes in BlobStore as a normal paper file, and previews raster assets. CSV assets feed the plot builder. SVG upload remains disabled because same-origin active SVG content is not yet covered by an asset sanitization policy.

Local builders generate ordinary editable LaTeX/BibTeX for tables, figures, equations/alignment/matrices/cases, pgfplots CSV plots, algorithmic algorithms, listings code, bibliography entries, and theorem-like environments. Builder preview and insertion are browser-only; no builder server endpoint writes source. Bibliography insertion opens the selected `.bib` Yjs room before appending. Generated snippets, autocomplete, citations, references, symbols, and builders all use explicit Writer-attributed `Y.Doc.transact` origins.

Package awareness comes from analyzer-reported `\usepackage` requests. Builders show `Available` or `Requires package: ...` for booktabs, graphicx, pgfplots, algorithm, and listings. S6 never edits a preamble or template-managed file automatically. Optional package insertion and subfigure generation are deferred.

## Security and scope

Mentor/Admin frontends remain non-mutating with respect to source. S6 adds no runtime CDN, shell execution, minted/shell escape, AI writing, WYSIWYG layer, restore graph, legacy migration, or M7 change. Compilation remains explicit/manual; durable saves and intelligence refreshes do not submit builds.
