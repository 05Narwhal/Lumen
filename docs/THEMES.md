# Making Lumen themes

A theme is a `.lumentheme` file: a zip that changes how Lumen looks, and may move things around,
but never what it does. Themes run no code, ask for no permissions and need no signature. Lumen
checks every byte before it uses it: tokens are parsed and written back by Lumen, and `theme.css`
passes a strict checker that only knows Lumen's published *parts*. Plugins can't carry themes
(plugin API 1.4); see [PLUGINS.md](PLUGINS.md) for plugins.

The authority for every rule is the code: `photos-plugins` (`theme_pkg.rs`, `theme_css.rs`,
`theme.rs`). If this page and Lumen disagree, Lumen is right.

## The package

```
theme.toml             the theme (required)
theme.css              layout and shape on Lumen's parts (optional; gives the Layout badge)
fonts/*.woff2          at most 2 families, 512 KB each (licence text may sit beside them as .txt)
images/*.png|jpg|webp  at most 2 MB each, used by theme.css
icon.png               at most 256 x 256
README.md              screenshots/*      LICENSE, NOTICES
```

Anything else (code, HTML, scripts, other archives) is refused. At most 20 MB. The path rules are
the plugin package's: relative, `/`-separated, no `..`, no links, no names differing only in case.

## theme.toml

```toml
id = "studio.example.nocturne"   # 3-64 of a-z, 0-9, "." and "-"
name = "Nocturne"                # at most 40 characters
version = "1.0.0"                # semver
author = "Example Studio"
description = "Inky violet."     # at most 140 characters
license = "CC-BY-4.0"            # SPDX; LicenseRef-* needs LICENSE
lumen = ">=26.2.0"
homepage = "https://example.com" # optional, shown as text
base = "darkroom"                # gallery | darkroom | paper | glass
default_scheme = "dark"          # light | dark: used the first time it is picked

[light]
accent = "oklch(52% 0.16 300)"

[dark]
bg-base = "oklch(15% 0.02 300)"

[font]                           # optional heading font
file = "fonts/heading.woff2"
weight = 600
style = "normal"

[listing]                        # optional
screenshots = ["screenshots/library.png"]
tags = ["dark", "layout"]        # dark | light | layout | colourful
```

The token tables, value forms and contrast checks are the ones plugin themes had: colours as
`oklch()`, `#hex`, `rgb()` or `hsl()`; lengths in `px` within each token's range; shadows of up to
three layers; `wash` of up to three radial gradients; no `url()`, `var()`, `;` or `}`. Tokens:
`bg-base`, `bg-raised`, `bg-sunken`, `develop-stage-bg`, `bg-sidebar`, `bg-overlay`, `fg-primary`,
`fg-muted`, `fg-subtle`, `fg-faint`, `accent`, `accent-hover`, `accent-fg`, `accent-soft`,
`border`, `border-width`, `scrollbar-thumb`, `radius-sm`, `radius-md`, `radius-lg`, `radius-pill`,
`tile-gap`, `shadow-sm`, `shadow-md`, `shadow-lg`, `blur`, `wash`. Text pairs need 4.5:1 in light
and dark (`fg-*` on every surface, `accent` on `bg-base` / `bg-raised`, `accent-fg` on the accent);
a theme that fails is refused, naming the pair (Developer mode installs it anyway).

## Parts

Lumen marks its layout with `data-part` attributes. These are the contract: a theme may style
them, and only them.

| Part | What it is |
| --- | --- |
| `shell` | The whole window: a CSS grid with named areas `titlebar`, `sidebar`, `main` |
| `titlebar` | The 38 px title bar (its window controls are protected) |
| `sidebar` | The navigation; once it is wide (a row) it shows the main entries and Settings along the row, without the album and device trees |
| `sidebar-item` | One entry in the sidebar |
| `main` | The page beside the sidebar |
| `toolbar` | A page's or the viewer's toolbar |
| `header` | A page header |
| `grid` | A photo grid |
| `tile` | One photo tile |
| `viewer` | The photo viewer overlay |
| `panel` | A side panel (info, Develop) |
| `card` | A card (Home, store, settings) |
| `button` | A button |
| `input` | A text field or search box |
| `statusbar` | A status line |

**Protected parts** can't be selected at all, and no rule reaches anything inside them:
`window-controls`, `dialog`, `confirm`, `toast`, `menu`, `install-sheet`, `palette`. A theme can't
hide, cover, move, relabel or make click-through a window control, a confirmation, the undo toast,
a menu, an install sheet or the palette.

Moving the sidebar to the top:

```css
[data-part="shell"] {
  grid-template-areas: "titlebar" "sidebar" "main";
  grid-template-columns: 1fr;
  grid-template-rows: auto auto 1fr;
}
[data-part="sidebar"] { flex-direction: row; }
```

## theme.css

Lumen reads it with its own checker and writes it back, scoped to the theme
(`[data-plugin-theme="<id>/theme"] …`) and kept off the protected parts. A refusal names the line
and the rule, for example `theme.css line 12: display: none isn't allowed … (hiding)`.

- **Selectors**: compounds of `[data-part="…"]`, optionally with `:hover`, `:active`,
  `:focus-visible`, `:focus-within`, `:first-child`, `:last-child`, joined by a space, `>`, `+` or
  `~`. No tags, classes, ids, `*`, other attributes or pseudo-elements.
- **Properties**: layout (`display`, `flex*`, `grid*`, `gap`, `order`, `align-*`, `justify-*`,
  `place-*`), box model (`width`, `height`, their `min-` / `max-`, `aspect-ratio`, `margin*`,
  `padding*`, `box-sizing`, `overflow*`), position (`position` except `fixed`, `top`, `right`,
  `bottom`, `left`, `inset*`), `border*`, `outline*`, `box-shadow`, `background*`, `color`,
  `font-family`, `font-size`, `font-weight`, `font-style`, `letter-spacing`, `line-height`,
  `text-transform`, `text-align`, `text-shadow`, `filter`, `backdrop-filter`, `transform`,
  `transform-origin`, `translate`, `scale`, `rotate`, `opacity`, `mix-blend-mode`, `isolation`,
  `object-fit`, `transition*`, `animation*`.
- **Functions**: colours (`rgb`, `hsl`, `oklch`, `oklab`, `color-mix`), `var(--<token>)` for
  Lumen's tokens only, `calc`, `min`, `max`, `clamp`, gradients, `repeat`, `minmax`,
  `fit-content`, transforms, filters, `cubic-bezier`, `steps`, and `url()` for the package's own
  files.
- **At-rules**: `@media` on width, height, orientation and `prefers-*`; `@keyframes`;
  `@font-face` with `font-family`, `src: url(fonts/….woff2)`, `font-weight`, `font-style`,
  `font-display` (declare fonts before the rules that use them). Font and animation names are
  renamed per theme, so they never replace Lumen's own.

**Refused, by rule name**: `remote-url` and `data-url` (any web or `data:` address), `url` (a
path that isn't the package's own font or image, or a file that isn't what its name says),
`import` (`@import`), `at-rule` (any other at-rule), `content` (`content:` or text in a value),
`protected` (a protected part), `unknown-part`, `selector`, `property` (anything not listed,
including `visibility`, `pointer-events`, `z-index` and custom properties), `hiding`
(`display: none`, `opacity` below 0.5, a scale below 0.5, text below 9 px), `off-screen` (an
offset, margin or translate below -64 px, or any negative non-px one), `fixed`
(`position: fixed`), `container` (`opacity`, filters, transforms or `position` on `shell` or
`titlebar`, which hold the window controls), `important` (`!important`), `escapes` (`\`), `var`
(a variable that isn't a Lumen token), `function`, `font` (more than two families, or a family
the theme didn't declare), `media`, `size` (over 128 KB), `syntax`.

## Assets

`theme.css` names files by relative path (`url(images/grain.png)`, `url(fonts/serif.woff2)`);
Lumen checks the file is in the package and is what its name says, and rewrites the address to
`photos://theme-asset/<id>/<path>`. The page never loads anything from the web.

## Installing, trying and picking

Installed themes live in `themes/installed/<id>/` under Lumen's app data folder and are picked in
Settings → Appearance like the four built-in ones (a miniature painted from its tokens, a *Layout*
badge when it has a `theme.css`, *Remove* with the 6 s undo). A `.lumentheme` opened from a file
(double-click, or *Install a theme from file…* in Appearance) shows an install sheet: name, author,
licence, a miniature, the Layout badge, and no permission list, since a theme asks for none.
*Try* in the store downloads the theme, checks
it the same way, and shows it without installing; it reverts when you leave or after 30 s unless
you keep it.

## Packing

```sh
node scripts/plugins/pack-theme.mjs <folder> [--out <file>]
```

writes `plugins/store/dist/<folder-name>.lumentheme` (git-ignored; the dev store reads it), the
same bytes every time, after the same path rules. `themes/examples/nord/` is a complete example.

## The store

The index's `themes[]` lists a theme like a plugin, without `permissions` / `kinds`, plus `tags`
(`dark`, `light`, `layout`, `colourful`) and `tokens: {light, dark}`, the swatches the store's
miniature is painted from (`bg`, `surface`, `sidebar`, `fg`, `fg2`, `accent`, `placeholder`,
`radius`, `gap`); `featuredTheme` names the one the Themes tab leads with. Both are optional, so
an older index still reads.

```sh
node plugins/store/add-theme.mjs <file.lumentheme> --url <release asset url> [--featured] [--tags dark,layout]
```

takes id, name, author, version, size and blake3 from the package and the tokens from
theme.toml, anything it doesn't set from its base (`plugins/store/base-swatches.json`, written by
a Rust test). *Try* and *Get* refuse a package whose blake3 isn't the index's.

Commands: `theme_try(storeId)`, `theme_try_end()`, `theme_keep()`, `theme_install(path | storeId)`,
`theme_remove(id)`, `theme_apply(id)` (sets `appearance.theme` to `{plugin: id, theme: "theme"}`
and sends `app://settings-changed` with the settings, then `plugins://changed`).

## Dev store

To see unpublished packages in the store: set `LUMEN_STORE_DIR` to a folder holding `index.json`
(usually `plugins/store/`), or turn on Developer mode and pick it under Settings → Plugins → *Use
a local store folder…*. Lumen then reads the index, packages and images from disk: a URL's file
name is looked up in `dist/`, then the folder itself, and still held to the index's size and
blake3 (re-run `add-plugin.mjs` / `add-theme.mjs` after packing). A release build ignores the
environment variable unless Developer mode is on.
