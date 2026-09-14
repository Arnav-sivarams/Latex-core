# Writer guide

## Document details

For a Team report, use the compact **Document details** toolbar action to view its Front Matter Pack, status, section choices, automatic metadata, and custom fields. The Team Leader can toggle optional sections and save fields permitted by the pack. Required sections stay enabled. Regular Team Writers have the same view without editing controls; institution-assigned Front Matter cannot be removed by a Writer.

Saving document details flushes current collaboration, safely escapes all entered text, atomically rebuilds hidden Front Matter files, and adds one workspace revision/history boundary. It never compiles. These generated files never appear in the file tree or enter an editable Yjs room. See [Front Matter Packs](FRONT_MATTER_PACKS.md).

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

The file-sidebar **Upload image** (`+`) action captures the currently open report before opening the picker and defaults to `assets/<filename>`. Every report has its own file manifest, so two reports—including two reports for the same Team—may each contain a different `assets/diagram.png`. The underlying immutable BlobStore may deduplicate identical bytes, but listing, preview, replacement, deletion, versions, export, compilation, and authorization remain report-local. If the path already exists, choose **Replace**, **Rename**, or **Cancel**; replacement keeps the stable file ID and creates a new report revision. PNG/JPEG uploads are limited to 1 MiB and must decode successfully with dimensions no greater than 8192×8192. Existing paths such as `images/chart.png` remain unchanged, appear as **Images (legacy)** with a truthful tooltip, and continue to compile. The sidebar no longer contains an Outline tab.

The Insert menu has separate long-table and algorithm choices. **Long Table Builder** emits a bare `longtable` with repeated-heading markers; it never wraps the result in `table`, `minipage`, or `resizebox`, and it reports that `longtable` is incompatible with a two-column layout. The Algorithm builder defaults to `algorithm` + `algpseudocode` commands such as `\State`; **Algorithmic Builder** uses the classic `algorithm` + `algorithmic` family and uppercase commands such as `\STATE`. Add the packages in the document preamble through an editable Writer action when the builder reports them missing.

**Editor settings** changes only the signed-in account's source-editor font size (12–26 px) and Light/Dark editor theme. Reconfiguration keeps the active collaboration document, undo history, review decorations, and the surrounding light application shell intact.

The workspace keeps Papers/Files, source, and PDF as three independently scrolling panes. Save, Compile, Insert, Math, Problems, History, and Reviews are available from the compact toolbar. Problems, immutable versions/revert controls, and active/resolved comments open in temporary drawers. File Rename, Delete, and Set Main live in the selected file's ellipsis or right-click menu.

Math is a searchable first-class palette (`∑`) covering common symbols, descriptions, components, and equation/matrix templates. Selection inserts through the same Writer-local Yjs transaction as the existing safe builders. `Ctrl/Cmd+P` opens files, `Ctrl/Cmd+K` opens commands, and a Team Leader can use `Ctrl/Cmd+Shift+R` to send an eligible paper for review.
