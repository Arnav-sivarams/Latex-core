# Writer guide

## Document details

For a Team report, use the compact **Document details** toolbar action to view its Front Matter Pack, status, section choices, automatic metadata, and custom fields. On first use the Leader is prompted once when required information is missing. Institutional Student, Team, Guide, department, School, semester, and year values are shown from their authoritative records; the form asks for project-specific or genuinely missing values. Completion persists and the panel can always be reopened. The Team Leader can toggle optional sections and save fields permitted by the pack. Required sections stay enabled. Regular Team Writers have the same view without editing controls; institution-assigned Front Matter cannot be removed by a Writer.

Saving document details flushes current collaboration, safely escapes values only while rendering, atomically rebuilds managed Front Matter files, and adds one workspace revision/history boundary. It never compiles. The generated project-scoped path is `.latex-core/frontmatter/Front-Matter.tex`; it is not editable or shown in the ordinary tree, but it is included in authorized immutable exports. Existing `metadata.tex`/`frontmatter.tex` pack integration remains in place. See [Front Matter Packs](FRONT_MATTER_PACKS.md).

Project metadata stores an executive summary (separate from abstract unless a pack explicitly maps it), project type, structured datasets, literal source-code snippets, and structured publications. Publications can be inserted as categorized `\bibitem` entries only at a cursor already inside an existing `thebibliography` environment.

Writers use `/write` for personal papers and assigned Team reports.

## Papers and files

- Create and own personal papers from the Writer paper list.
- Open only Team reports to which an Admin assigned you.
- Create, rename, delete, and select the main file when the Team lifecycle and current file policy permit it.
- File identity is stable across renames and version history.

The five policies are `EDITABLE`, `CONTENT_READ_ONLY`, `STRUCTURE_LOCKED`, `TEMPLATE_MANAGED`, and `HIDDEN_SYSTEM`. The server enforces them on HTTP and collaboration connections. Hidden system files are omitted from the normal tree.

## Realtime, offline, and saving

Source changes synchronize through durable collaboration rooms. **Save** and `Ctrl/Cmd+S` request durable CRDT synchronization; they do not compile or create a checkpoint. The indicator distinguishes Saving, Saved, Offline, and Reconnecting states.

Offline state is stored per paper epoch. After a governed restoration, changes from the previous epoch are preserved locally and are not merged into the restored paper. Use **Copy recovery text** to retrieve that buffer.

## Build and PDF

PDF compilation is manual. Editing, autosave, collaboration, file/image changes, Front Matter saves, history operations, opening, and reconnecting never submit a build. Choose **Compile** or press `Ctrl/Cmd+Enter`; this first flushes permitted collaboration changes, captures an immutable exact-state version, and submits the bounded idempotent job. The status shows the real Queued, Compiling, Compiled, or Failed state.

Before the first successful build, the PDF pane says **Compile to generate a PDF.** If source or metadata changes later, the last successful PDF remains downloadable as historical output but is labelled **PDF is out of date — Compile to refresh.** It is not represented as current.

## History and restoration

History contains immutable versions. A Writer may directly restore an owned personal paper after confirmation; the old head is first retained as a safety version and restoration creates a new head.

For a Team report, a regular Writer chooses **Request Revert**, selects a version, optionally enters a reason, and submits it to the Team Leader. The Leader can reject or safely apply the request. A Leader may also directly choose **Revert** after explicit confirmation. Each applied revert first records `PRE_RESTORE_SAFETY`, advances the document epoch, and creates a new head without deleting later history.

## Reviews and productivity

The Team Leader can use **Send for review** only when the exact current source and metadata have a successful matching PDF; otherwise the server says **Compile the latest changes before sending for review.** Sending never compiles. Once the round opens, every Writer—including the Leader—is read-only for source, files, Front Matter, restoration, compilation, and published-feedback Apply/Done actions. Reading, navigation, PDF/source download, and published feedback remain available. Regular Writers see **Under review — your Team Leader can end the review.** The Leader retains only **End review** as a report-changing action. Ending the round restores each Writer's normal role and file-policy permissions; it does not grant new permissions or publish/delete private Mentor drafts.

Writers receive feedback only after a Mentor uses **Push review**. With multiple Mentors, one push publishes only that Mentor's feedback and does not unlock the report; the round closes only after the final required Mentor pushes or the Leader chooses **End review**. The **Reviews** toolbar count includes published unresolved feedback after closure. Selecting an item opens its stable file identity at the anchored range; if source drift prevents a trustworthy mapping, the original excerpt and reviewed PDF location are shown instead of highlighting unrelated text. Once editing resumes, **Done** resolves one highlight and **Apply** uses the Writer-attributed suggestion flow.

The file-sidebar **Upload image** (`+`) action captures the currently open report before opening the picker and proposes the actual existing `images/` or `assets/` directory beneath a wrapper-aware content root. The complete report-local destination is shown and can be changed before upload. Every report has its own file manifest, so two reports—including two reports for the same Team—may each contain a different `assets/diagram.png`. The underlying immutable BlobStore may deduplicate identical bytes, but listing, preview, replacement, deletion, versions, export, compilation, and authorization remain report-local. If the path already exists, choose **Replace**, **Rename**, or **Cancel**; replacement keeps the stable file ID and creates a new report revision. PNG/JPEG uploads are limited to 1 MiB and must decode successfully with dimensions no greater than 8192×8192. Existing paths such as `images/chart.png` remain unchanged and continue to compile. After upload, the optional figure insertion keeps its caption below the figure. The sidebar no longer contains an Outline tab.

The single **Insert** catalogue contains each function once. It includes table/long table, figure, `listings` code blocks, inline/display math, a searchable keyboard-accessible symbol grid, publications, algorithms, citations, references, lists, and source comment/uncomment. All selection-based actions preserve the editor range across menus and use one Yjs transaction, so normal collaboration and undo apply. Inline math avoids adding a second delimiter when the cursor is already in inline math. Source comment/uncomment applies TeX `%` prefixes to every selected line, including partial and blank-line selections.

Table captions are generated above the `tabular`; figure captions stay below the image. Table column widths and minimum row height accept positive bounded LaTeX dimensions. Width applies to an entire column and minimum height to an entire row—the UI does not promise impossible independent cell geometry. **Long Table Builder** emits a bare `longtable` with repeated-heading markers; it never wraps the result in `table`, `minipage`, or `resizebox`, and reports two-column incompatibility. Code uses `listings`, never `minted` or shell escape.

**Editor settings** changes only the signed-in account's source-editor font size (12–26 px) and Light/Dark editor theme. Reconfiguration keeps the active collaboration document, undo history, review decorations, and the surrounding light application shell intact.

The workspace keeps Papers/Files, source, and PDF as three independently scrolling panes. Save, Compile, Insert, Math, Problems, History, and Reviews are available from the compact toolbar. Problems, immutable versions/revert controls, and active/resolved comments open in temporary drawers. File Rename, Delete, and Set Main live in the selected file's ellipsis or right-click menu.

Math is a searchable first-class palette (`∑`) covering common symbols, descriptions, components, and equation/matrix templates. Selection inserts through the same Writer-local Yjs transaction as the existing safe builders. `Ctrl/Cmd+P` opens files, `Ctrl/Cmd+K` opens commands, and a Team Leader can use `Ctrl/Cmd+Shift+R` to send an eligible paper for review.
