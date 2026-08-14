# Templates

Users can import an authorized LaTeX ZIP directly through **New → Import Project**. Administrators can maintain a reusable server library:

```sh
latex-core template add ./ieee.zip --name "IEEE Conference" --main conference.tex
latex-core template list
latex-core template remove "IEEE Conference"
```

`--description` is optional. ZIP validation is identical for template installation and user project import. A template is copied into a new, user-owned durable project, so later edits never change the source template and deleting a template does not delete projects created from it.

Use authorized ZIPs for IEEE, ACM, Springer, thesis, CV, or local lab workflows. Project-local `.cls`, `.sty`, `.bst`, and `.bib` files stay inside the project. Templates do not modify the frozen M7 compiler environment or install TeX packages globally.
