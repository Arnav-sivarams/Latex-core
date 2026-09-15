# Templates

Users can import an authorized LaTeX ZIP directly through **New → Import Project**. Administrators can maintain a reusable server library:

```sh
latex-core template add ./ieee.zip --name "IEEE Conference" --main conference.tex
latex-core template list
latex-core template remove "IEEE Conference"
```

`--description` is optional. ZIP validation is identical for template installation and user project import. A template is copied into a new, user-owned durable project, so later edits never change the source template and deleting a template does not delete projects created from it.

Use authorized ZIPs for IEEE, ACM, Springer, thesis, CV, or local lab workflows. Project-local `.cls`, `.sty`, `.bst`, and `.bib` files stay inside the project. Templates do not modify the frozen M7 compiler environment or install TeX packages globally.

## Front Matter arrangement

Admin template import/edit records one explicit arrangement:

- **Report content only** captures project metadata but forces no Front Matter pages.
- **Separate files** uses the verified Front Matter marker and existing modern/VIT pack materialization.
- **One complete source** enables the same metadata GUI. A supported institutional source declares at least one known registry macro, then places the exact `% LATEX_CORE_SINGLE_SOURCE_BINDINGS` marker after its placeholder definitions and before `\begin{document}`. Each report copy replaces that marker once with its managed definitions input; the immutable uploaded template is never edited.

The known allowlisted registry covers the institutional title, Student name/registration, Guide, programme/course, semester/year, School, Dean and HoD/department macros already supported by the legacy resolver. The current release does not transform arbitrary single-file TeX or inject guessed macros. A source with a missing/misplaced marker or no known declaration receives a compatibility warning and remains metadata-only. Uploaded originals, document environments, Front Matter pages, and bibliography arrangements are never duplicated by blind text replacement.
