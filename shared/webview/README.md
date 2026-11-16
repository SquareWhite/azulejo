# Shared webview assets

The HTML, CSS and JavaScript around every preview tile, shared by all shells.

The core embeds these files (`include_str!` in `core/crates/preview`) and
sends each preview as one self-contained HTML document, in `preview.updated`
events and `preview.get` replies. Shells never load the files themselves.

- `preview.html`: the page skeleton. Its content security policy allows only
  inline styles, the inline script, and the images, media and fonts the
  document references (`file:` and `data:`, plus remote images).
- `preview.css`: typography, light and dark colours, code blocks, the
  line-number gutter, notices. The core appends the syntax colours (`syn-*`
  classes), generated from two syntect themes.
- `preview.js`: `azulejoPreview.swap(html)`. When an update arrives for the
  file a tile already shows, the shell calls it with the event's `html`
  instead of reloading, so the reader keeps their scroll position (or stays at
  the bottom of a growing file). It returns `false` when the page isn't one of
  ours; the shell then loads the document from scratch.

Native kinds (`image`, `video`, `audio`, `pdf`) carry no HTML: shells show
`path` natively (QuickLook on macOS). Links in markdown to local files arrive
as `file://` URLs. Load the page with read access to the file's folder
(WKWebView `loadHTMLString(_:baseURL:)` with a `file://` base, or
`loadFileURL(_:allowingReadAccessTo:)`) so images resolve.

Nothing here scripts browser tiles.
