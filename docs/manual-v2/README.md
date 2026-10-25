# Building the CharlotteOS manual

The manual embeds the architecture figures as SVG through LaTeX's `svg`
package. The checked-in SVGs are generated from the Mermaid blocks in
[`../figures.md`](../figures.md).

Regenerate them after changing a Mermaid block:

```sh
./docs/manual-v2/render-figures.sh
```

This requires Mermaid CLI (`mmdc`) and Inkscape. It regenerates the SVG sources
and pre-renders the `svg-inkscape/` PDF cache that the `svg` package consumes.
Build the manual from this directory with shell escape enabled:

```sh
cd docs/manual-v2
pdflatex --shell-escape charlotte.tex
pdflatex --shell-escape charlotte.tex
```

The second pass resolves the table of contents and clickable cross-references.
For TeXShop, select or create an engine that passes `--shell-escape` to
`pdflatex`; SyncTeX continues to work with the SVG figures.

Chapter 5 includes the [reconnecting client example](../../crates/catten-services/examples/reconnecting_client.rs)
directly through marked `lstinputlisting` ranges. Keep the range markers intact
when editing the example; its source must compile without additions to the
listings. Build and host-test commands, prerequisites and recovery limits are in
the [client guide](../guides/reconnecting-clients.md).

The [18 October 2026 source comparison](../reports/audits/2026-10-18-documentation-reality-sweep.md)
records the latest manual sweep, corrections and verification limits. Short
architectural listings are explanatory sketches unless identified as included,
compiled source. Keep current API and validation claims tied to source or dated
evidence; a PDF rebuild is not a fresh runtime qualification.

The **Configuration** chapter and matching [Markdown guide](../guides/configuration.md)
describe current configuration inputs, central compile-time resource profiles,
related capacity choices, qualification, and the boundary to future boot resources.
The [resource-policy reference](../reference/resource-policy.md) inventories exact
units and default values.
