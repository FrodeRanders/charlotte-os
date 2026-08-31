# Building the CharlotteOS manual

The manual embeds the architecture figures as SVG through LaTeX's `svg`
package. The checked-in SVGs are generated from the Mermaid blocks in
[`../figures.md`](../figures.md).

Regenerate them after changing a Mermaid block:

```sh
./docs/manual-v2/render-figures.sh
```

This requires Mermaid CLI (`mmdc`). Build the manual from this directory with
shell escape enabled so the `svg` package can invoke Inkscape:

```sh
cd docs/manual-v2
pdflatex --shell-escape charlotte.tex
pdflatex --shell-escape charlotte.tex
```

The second pass resolves the table of contents and clickable cross-references.
For TeXShop, select or create an engine that passes `--shell-escape` to
`pdflatex`; SyncTeX continues to work with the SVG figures.
