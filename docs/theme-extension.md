# Theme extension proposal: code, diff and data roles

From gitterrace (the read-only git browser in this repo) to tendir's theme
system. tendir's themes (Kandel, Elz, Hochburg, Schauinsland) define ink,
accent, signals, pigments and a few fills for a task app. A product that shows
code, diffs and charts needs roles tendir does not have. gitterrace solves
them from tendir's own tokens and rules, so they would fit every theme
unchanged. This page is the proposal: the missing roles, the rule for each,
the solved values for all four themes, and where they would live in tendir.

Reproduce: `python3 scripts/solve-theme.py` (report and every floor),
`--markdown` (the tables below), `--check` (fails on a drift or a floor).
Input is `tendir/docs/generated/theme-tokens.json` plus `selection_wash` from
`tendir-types/src/theme.rs` (the JSON export omits it; the export should
carry it). No dependencies: OKLCH, CIEDE2000, WCAG, APCA and the Machado
CVD matrices are vendored in the script.

## Missing roles and their rules

Shares are sRGB alpha composites, as a browser and AppKit draw them.

| Role | gitterrace token | Rule |
|---|---|---|
| Page window | `--n-0` / `--n-1` | Light: white surface, page bg primary ink at 2%. Dark: window L 0.185 grey, surface ink at 3%. tendir has no web window; surfaces stay near chroma 0 (under 0.003). |
| Soft, code surface | `--n-2` | Primary ink over the window at 4% light, 5% dark (the chipFillQuiet construction). |
| Hover, heat 0 | `--n-3` | Ink at 7% light, 8% dark. |
| Line, chart grid | `--n-4` | Ink at 13% light, 12% dark. |
| Line strong, axis, scrollbar thumb, image checker | `--n-5` | Ink at 25% light, 22% dark. |
| Header band | `--header` | One surface step below the page (the website's `--forest`), selection-wash hue at the light cast (CIEDE2000 2.0) in both modes. |
| Raised (popover, dialog) | `--raised` | Light: the surface. Dark: ink at 8% over the window, lifting off the page as system popovers do. |
| Scrim | `--scrim` | tendir's `SCRIM` veil, black at 30% in both modes: it only darkens, never greys a dark page. |
| Selected row | `--accent-soft` | tendir's selectionWash at 12% over the surface, its chroma raised (L, H held) until the cast reaches 2.0 light, 4.0 dark. |
| Text selection | `--selection` | tendir's choiceFill: accent at 24% light, 35% dark. |
| Focus ring | `--focus` (`--ring` alpha kept) | accentInk, 3:1 on every surface. |
| Diff row added / deleted / warning | `--add-bg` `--del-bg` `--warn-bg` | The signal at 10% light, 14% dark over the surface. Must clear the themed-fill cast (2.0 / 4.0) and keep the signal at 4.5:1 and secondary ink at 4.5:1 on it. |
| Diff word highlight | `--add-hl` `--del-hl` | The signal at 24% light, 30% dark: one pill step above its row, primary ink 7:1 on it. |
| Heat ramp | `--seq-0..4` | seq-0 the hover surface, seq-4 accentInk, steps between at equal OKLCH L at the accent hue with chroma rising linearly; monotonic L, adjacent steps at least CIEDE2000 5. |
| Categorical series | `--cat-1..8` | Entity pigments Blue, Orange, Green, Pink, Teal, Red, Yellow; tendir's pigment rule (keep L unless under 3:1, chroma in the theme band); cat-8 "other" is line strong. |
| Syntax | `--syn-*` | comment: tertiary ink; keyword Purple, string Green, constant Blue, entity accentInk, tag Teal, param Orange (entity pigments); pigment rule at 4.5:1 on the code surface. Schauinsland: secondary ink. |
| Mermaid, charts | (uses the above) | Diagrams take surface, line, ink and accent tokens; no own colours. |

## Weak spot: add vs delete under colour-vision deficiency

The signals are tendir's and separate at CIEDE2000 7.5 light, 15.5 dark
under the worst of protanopia, deuteranopia and tritanopia (tendir's own
minimum is 7.4). The row tints are those signals at 10 to 14%, so they keep
only 0.6 light / 2.6 dark: a wash that calm cannot carry a distinction. Moving
the rows apart would need either a lightness offset (a deleted row darker than
an added one, which reads as importance) or more share (loud rows), both
against tendir's rules. So the rule stays and the diff follows tendir's
Differentiate Without Color: every row carries a `+`/`-` glyph and a gutter
number in the signal colour, counts carry their sign.

## Solved values

#### Light

Each cell: hex, OKLCH L C H, then contrast against the pair named in the second column.

| Token | Role (contrast vs) | Kandel | Elz | Hochburg | Schauinsland |
|---|---|---|---|---|---|
| `--n-0` | surface (vs `n-9`) | `#ffffff` 1.000 0.000 - 15.1:1 | `#ffffff` 1.000 0.000 - 15.0:1 | `#ffffff` 1.000 0.000 - 15.0:1 | `#ffffff` 1.000 0.000 - 15.1:1 |
| `--n-1` | page bg (vs `n-9`) | `#fbfbfb` 0.988 0.000 - 14.6:1 | `#fbfbfb` 0.988 0.000 - 14.5:1 | `#fbfbfa` 0.988 0.001 - 14.5:1 | `#fbfbfb` 0.988 0.000 - 14.6:1 |
| `--n-2` | soft / code surface (vs `n-9`) | `#f6f6f7` 0.973 0.001 - 13.9:1 | `#f6f6f6` 0.973 0.000 - 13.8:1 | `#f7f6f6` 0.974 0.001 - 13.9:1 | `#f6f6f6` 0.973 0.000 - 14.0:1 |
| `--n-3` | hover, seq-0 (vs `n-9`) | `#eff0f0` 0.954 0.001 - 13.2:1 | `#eff0f0` 0.954 0.001 - 13.1:1 | `#f0f0ef` 0.955 0.001 - 13.2:1 | `#f0f0f0` 0.955 0.000 - 13.3:1 |
| `--n-4` | line, chart grid (vs `n-0`) | `#e2e3e4` 0.915 0.002 - 1.3:1 | `#e2e3e3` 0.915 0.001 - 1.3:1 | `#e3e3e2` 0.916 0.001 - 1.3:1 | `#e3e3e3` 0.916 0.000 - 1.3:1 |
| `--n-5` | line-strong, axis, scrollbar thumb (vs `n-0`) | `#c8c9cb` 0.836 0.003 - 1.7:1 | `#c7cac8` 0.836 0.004 - 1.7:1 | `#cac9c7` 0.836 0.003 - 1.7:1 | `#c9c9c9` 0.836 0.000 - 1.7:1 |
| `--n-8` | ink between primary and secondary (vs `n-1`) | `#363c44` 0.354 0.016 256 10.8:1 | `#343e38` 0.353 0.017 159 10.7:1 | `#403b33` 0.355 0.015 80 10.7:1 | `#3b3b3b` 0.352 0.000 - 10.8:1 |
| `--header` | header band (vs `n-9`) | `#edf0f4` 0.954 0.006 - 13.2:1 | `#eef1ef` 0.955 0.004 - 13.2:1 | `#f2f0ec` 0.956 0.006 - 13.2:1 | `#f0f0f0` 0.955 0.000 - 13.3:1 |
| `--raised` | popover, dialog (vs `n-9`) | `#ffffff` 1.000 0.000 - 15.1:1 | `#ffffff` 1.000 0.000 - 15.0:1 | `#ffffff` 1.000 0.000 - 15.0:1 | `#ffffff` 1.000 0.000 - 15.1:1 |
| `--accent-soft` | selected row (vs `n-9`) | `#eceff3` 0.951 0.006 - 13.1:1 | `#edf0ee` 0.952 0.004 - 13.0:1 | `#f1eeea` 0.950 0.006 - 13.0:1 | `#efefef` 0.952 0.000 - 13.2:1 |
| `--selection` | text selection (vs `n-9`) | `#d1d2db` 0.866 0.012 281 10.0:1 | `#cad4db` 0.864 0.015 238 9.9:1 | `#d9cfd5` 0.864 0.014 341 9.9:1 | `#d3d3d3` 0.867 0.000 - 10.1:1 |
| `--focus` | focus ring (vs `n-0`) | `#5b5e8d` 0.499 0.075 281 6.1:1 | `#1265a5` 0.495 0.126 248 6.1:1 | `#825275` 0.504 0.081 337 6.2:1 | `#262626` 0.269 0.000 - 15.1:1 |
| `--add-bg` | added row (vs `n-9`) | `#e9f0ee` 0.949 0.008 - 13.0:1 | `#e9f0ee` 0.949 0.008 - 12.9:1 | `#e9f0ee` 0.949 0.008 - 13.0:1 | `#e9f0ee` 0.949 0.008 - 13.1:1 |
| `--add-hl` | added word (vs `n-9`) | `#cadbd7` 0.877 0.019 180 10.5:1 | `#cadbd7` 0.877 0.019 180 10.4:1 | `#cadbd7` 0.877 0.019 180 10.5:1 | `#cadbd7` 0.877 0.019 180 10.5:1 |
| `--del-bg` | deleted row (vs `n-9`) | `#f7ebec` 0.950 0.013 12 12.9:1 | `#f7ebec` 0.950 0.013 12 12.9:1 | `#f7ebec` 0.950 0.013 12 12.9:1 | `#f7ebec` 0.950 0.013 12 13.0:1 |
| `--del-hl` | deleted word (vs `n-9`) | `#eccfd0` 0.879 0.033 15 10.3:1 | `#eccfd0` 0.879 0.033 15 10.3:1 | `#eccfd0` 0.879 0.033 15 10.3:1 | `#eccfd0` 0.879 0.033 15 10.4:1 |
| `--warn-bg` | warning row (vs `n-9`) | `#f3eee6` 0.951 0.012 80 13.0:1 | `#f3eee6` 0.951 0.012 80 13.0:1 | `#f3eee6` 0.951 0.012 80 13.0:1 | `#eeeeee` 0.949 0.000 - 13.0:1 |
| `--seq-1` | heat 1 (vs `n-0`) | `#c8cad8` 0.842 0.019 279 1.6:1 | `#bbcddf` 0.840 0.032 248 1.6:1 | `#d5c6d0` 0.842 0.021 338 1.6:1 | `#b8b8b8` 0.783 0.000 - 2.0:1 |
| `--seq-2` | heat 2 (vs `n-0`) | `#a2a5bf` 0.728 0.038 280 2.4:1 | `#87aacd` 0.725 0.064 249 2.4:1 | `#b99eb1` 0.729 0.041 337 2.4:1 | `#848484` 0.613 0.000 - 3.7:1 |
| `--seq-3` | heat 3 (vs `n-0`) | `#7d81a6` 0.614 0.056 280 3.8:1 | `#5388b9` 0.610 0.094 248 3.8:1 | `#9e7792` 0.616 0.061 338 3.8:1 | `#525252` 0.439 0.000 - 7.8:1 |
| `--seq-4` | heat 4 (vs `n-0`) | `#5b5e8d` 0.499 0.075 281 6.1:1 | `#1265a5` 0.495 0.126 248 6.1:1 | `#825275` 0.504 0.081 337 6.2:1 | `#262626` 0.269 0.000 - 15.1:1 |
| `--syn-comment` | comment (vs `n-2`) | `#6a727b` 0.548 0.017 251 4.5:1 | `#69736d` 0.545 0.015 160 4.5:1 | `#777067` 0.549 0.016 74 4.5:1 | `#717171` 0.549 0.000 - 4.5:1 |
| `--syn-keyword` | keyword (vs `n-2`) | `#513761` 0.385 0.076 312 9.4:1 | `#764481` 0.471 0.111 320 6.7:1 | `#74568c` 0.506 0.090 309 5.6:1 | `#505050` 0.431 0.000 - 7.5:1 |
| `--syn-string` | string (vs `n-2`) | `#127050` 0.485 0.098 164 5.6:1 | `#366e31` 0.485 0.110 142 5.7:1 | `#386d42` 0.486 0.089 148 5.7:1 | `#505050` 0.431 0.000 - 7.5:1 |
| `--syn-constant` | constant (vs `n-2`) | `#306693` 0.495 0.092 246 5.6:1 | `#00587a` 0.433 0.090 233 7.3:1 | `#4f6384` 0.497 0.058 260 5.6:1 | `#505050` 0.431 0.000 - 7.5:1 |
| `--syn-entity` | entity (vs `n-2`) | `#5b5e8d` 0.499 0.075 281 5.7:1 | `#1365a5` 0.495 0.126 248 5.7:1 | `#825275` 0.504 0.081 337 5.7:1 | `#262626` 0.269 0.000 - 14.0:1 |
| `--syn-tag` | tag (vs `n-2`) | `#1f6a72` 0.483 0.072 205 5.8:1 | `#006f66` 0.487 0.086 185 5.6:1 | `#186d6b` 0.487 0.076 193 5.7:1 | `#505050` 0.431 0.000 - 7.5:1 |
| `--syn-param` | parameter (vs `n-2`) | `#8c5528` 0.503 0.094 57 5.6:1 | `#915313` 0.505 0.110 60 5.6:1 | `#8e5417` 0.502 0.105 62 5.7:1 | `#505050` 0.431 0.000 - 7.5:1 |

#### Dark

Each cell: hex, OKLCH L C H, then contrast against the pair named in the second column.

| Token | Role (contrast vs) | Kandel | Elz | Hochburg | Schauinsland |
|---|---|---|---|---|---|
| `--n-0` | surface (vs `n-9`) | `#191a1a` 0.217 0.002 - 14.6:1 | `#191a19` 0.216 0.002 - 14.6:1 | `#1a1919` 0.215 0.002 - 14.7:1 | `#191919` 0.213 0.000 - 14.7:1 |
| `--n-1` | page bg (vs `n-9`) | `#131313` 0.187 0.000 - 15.6:1 | `#131313` 0.187 0.000 - 15.6:1 | `#131313` 0.187 0.000 - 15.5:1 | `#131313` 0.187 0.000 - 15.6:1 |
| `--n-2` | soft / code surface (vs `n-9`) | `#1d1e1e` 0.234 0.002 - 14.0:1 | `#1d1e1e` 0.234 0.002 - 14.0:1 | `#1e1e1d` 0.235 0.002 - 14.0:1 | `#1e1e1e` 0.235 0.000 - 14.0:1 |
| `--n-3` | hover, seq-0 (vs `n-9`) | `#242425` 0.261 0.002 - 13.0:1 | `#242524` 0.263 0.002 - 12.9:1 | `#252423` 0.261 0.002 - 13.0:1 | `#242424` 0.260 0.000 - 13.0:1 |
| `--n-4` | line, chart grid (vs `n-0`) | `#2c2d2e` 0.297 0.002 - 1.3:1 | `#2c2d2c` 0.296 0.002 - 1.3:1 | `#2e2d2b` 0.297 0.004 - 1.3:1 | `#2d2d2d` 0.297 0.000 - 1.3:1 |
| `--n-5` | line-strong, axis, scrollbar thumb (vs `n-0`) | `#414345` 0.382 0.004 - 1.8:1 | `#414342` 0.381 0.003 - 1.8:1 | `#444240` 0.380 0.004 - 1.8:1 | `#434343` 0.383 0.000 - 1.8:1 |
| `--n-8` | ink between primary and secondary (vs `n-1`) | `#cfd8e2` 0.878 0.017 251 12.9:1 | `#cedad3` 0.877 0.016 161 12.9:1 | `#ddd6cc` 0.879 0.016 77 12.9:1 | `#d7d7d7` 0.879 0.000 - 12.9:1 |
| `--header` | header band (vs `n-9`) | `#090d11` 0.157 0.011 248 16.3:1 | `#0a0e0c` 0.158 0.008 - 16.3:1 | `#0f0d08` 0.160 0.011 89 16.3:1 | `#0d0d0d` 0.159 0.000 - 16.3:1 |
| `--raised` | popover, dialog (vs `n-9`) | `#242425` 0.261 0.002 - 13.0:1 | `#242524` 0.263 0.002 - 12.9:1 | `#252423` 0.261 0.002 - 13.0:1 | `#242424` 0.260 0.000 - 13.0:1 |
| `--accent-soft` | selected row (vs `n-9`) | `#262c31` 0.289 0.012 243 11.8:1 | `#272c29` 0.287 0.009 160 11.9:1 | `#2e2a24` 0.287 0.012 78 11.9:1 | `#2a2a2a` 0.285 0.000 - 12.0:1 |
| `--selection` | text selection (vs `n-9`) | `#343643` 0.336 0.023 278 10.0:1 | `#293843` 0.332 0.028 240 10.1:1 | `#41323b` 0.337 0.026 342 10.1:1 | `#363636` 0.333 0.000 - 10.1:1 |
| `--focus` | focus ring (vs `n-0`) | `#9ca1d4` 0.723 0.074 280 7.0:1 | `#77a9d6` 0.717 0.085 246 7.0:1 | `#ca95ba` 0.732 0.080 338 7.1:1 | `#ebebeb` 0.940 0.000 - 14.7:1 |
| `--add-bg` | added row (vs `n-9`) | `#263330` 0.308 0.018 179 11.0:1 | `#26332f` 0.308 0.019 174 11.0:1 | `#27322f` 0.306 0.016 177 11.1:1 | `#26322f` 0.305 0.017 178 11.2:1 |
| `--add-hl` | added word (vs `n-9`) | `#354e49` 0.402 0.032 181 7.5:1 | `#354e48` 0.402 0.032 178 7.5:1 | `#364e48` 0.402 0.031 177 7.5:1 | `#354e48` 0.402 0.032 178 7.5:1 |
| `--del-bg` | deleted row (vs `n-9`) | `#392727` 0.294 0.027 19 11.8:1 | `#392726` 0.294 0.027 22 11.8:1 | `#3a2626` 0.293 0.030 19 11.8:1 | `#392626` 0.292 0.029 19 11.9:1 |
| `--del-hl` | deleted word (vs `n-9`) | `#5e3636` 0.382 0.058 20 8.6:1 | `#5e3635` 0.382 0.058 22 8.6:1 | `#5f3535` 0.381 0.061 20 8.6:1 | `#5e3535` 0.380 0.059 20 8.6:1 |
| `--warn-bg` | warning row (vs `n-9`) | `#352d20` 0.302 0.025 80 11.4:1 | `#352d1f` 0.302 0.026 81 11.4:1 | `#362c1f` 0.300 0.026 74 11.4:1 | `#313131` 0.313 0.000 - 10.9:1 |
| `--seq-1` | heat 1 (vs `n-0`) | `#3f404c` 0.375 0.020 282 1.7:1 | `#38434d` 0.377 0.022 246 1.7:1 | `#4a3e46` 0.379 0.021 338 1.7:1 | `#505050` 0.431 0.000 - 2.2:1 |
| `--seq-2` | heat 2 (vs `n-0`) | `#5c5f77` 0.492 0.038 279 2.8:1 | `#4c6378` 0.489 0.044 246 2.8:1 | `#72596b` 0.496 0.042 337 2.8:1 | `#808080` 0.600 0.000 - 4.5:1 |
| `--seq-3` | heat 3 (vs `n-0`) | `#7b7fa4` 0.607 0.056 280 4.5:1 | `#6185a6` 0.603 0.065 247 4.5:1 | `#9d7791` 0.615 0.060 338 4.6:1 | `#b4b4b4` 0.770 0.000 - 8.5:1 |
| `--seq-4` | heat 4 (vs `n-0`) | `#9ca1d4` 0.723 0.074 280 7.0:1 | `#77a9d6` 0.717 0.085 246 7.0:1 | `#ca95ba` 0.732 0.080 338 7.1:1 | `#ebebeb` 0.940 0.000 - 14.7:1 |
| `--syn-comment` | comment (vs `n-2`) | `#a1aab3` 0.734 0.017 248 7.1:1 | `#a1aca5` 0.734 0.016 158 7.1:1 | `#aea89e` 0.734 0.016 81 7.1:1 | `#a9a9a9` 0.735 0.000 - 7.1:1 |
| `--syn-keyword` | keyword (vs `n-2`) | `#b992d2` 0.719 0.100 312 6.4:1 | `#c28fd2` 0.722 0.110 318 6.5:1 | `#cfa3e3` 0.777 0.101 315 7.9:1 | `#c3c3c3` 0.817 0.000 - 9.5:1 |
| `--syn-string` | string (vs `n-2`) | `#50b291` 0.695 0.105 168 6.5:1 | `#7eae68` 0.699 0.110 136 6.5:1 | `#73af74` 0.697 0.105 145 6.4:1 | `#c3c3c3` 0.817 0.000 - 9.5:1 |
| `--syn-constant` | constant (vs `n-2`) | `#66a6dd` 0.704 0.105 246 6.4:1 | `#5fbfeb` 0.764 0.110 230 8.1:1 | `#85a4bc` 0.704 0.049 242 6.4:1 | `#c3c3c3` 0.817 0.000 - 9.5:1 |
| `--syn-entity` | entity (vs `n-2`) | `#9ca1d4` 0.723 0.074 280 6.7:1 | `#77a9d6` 0.717 0.085 246 6.7:1 | `#ca95ba` 0.732 0.080 338 6.8:1 | `#ebebeb` 0.940 0.000 - 14.0:1 |
| `--syn-tag` | tag (vs `n-2`) | `#4caeb8` 0.697 0.092 204 6.4:1 | `#4dbb9e` 0.720 0.110 173 7.1:1 | `#72ac9b` 0.699 0.066 174 6.4:1 | `#c3c3c3` 0.817 0.000 - 9.5:1 |
| `--syn-param` | parameter (vs `n-2`) | `#cf945f` 0.713 0.099 62 6.4:1 | `#ce9750` 0.715 0.110 71 6.5:1 | `#cf9458` 0.712 0.105 65 6.4:1 | `#c3c3c3` 0.817 0.000 - 9.5:1 |

#### Separation and casts

| Check | Kandel light | dark | Elz light | dark | Hochburg light | dark | Schauinsland light | dark |
|---|---|---|---|---|---|---|---|---|
| add-bg cast | 3.7 | 7.7 | 3.7 | 8.0 | 3.7 | 6.9 | 3.7 | 7.3 |
| del-bg cast | 5.6 | 10.1 | 5.6 | 10.1 | 5.6 | 11.0 | 5.6 | 10.6 |
| warn-bg cast | 4.1 | 8.1 | 4.1 | 8.5 | 4.1 | 8.6 | n/a | n/a |
| accent-soft cast | 2.2 | 4.1 | 2.0 | 4.2 | 2.2 | 4.2 | n/a | n/a |
| add/del CVD | 7.5 | 15.5 | 7.5 | 15.5 | 7.5 | 15.5 | 7.5 | 15.5 |
| add-bg/del-bg CVD | 0.6 | 2.6 | 0.6 | 2.6 | 0.6 | 2.6 | 0.6 | 2.6 |
| add-hl/del-hl CVD | 1.5 | 5.6 | 1.5 | 5.6 | 1.5 | 5.6 | 1.5 | 5.6 |
| seq min adjacent step | 10.7 | 11.1 | 11.5 | 11.7 | 11.3 | 12.1 | 13.2 | 13.2 |


## How it would slot into tendir

A design, not code in tendir.

1. **`tendir-types/src/theme.rs`**: a new `Surfaces` struct (window, bg,
   soft, hover, line, line_strong, header, raised) and a `Code` struct
   (add_row, add_word, del_row, del_word, warn_row, selection, syntax:
   `[(&str, Pair); 7]`, heat: `[Pair; 5]`, series: `[Pair; 8]`) on `Theme`,
   as `Pair` hex literals like every other token. Most entries are not new
   colour ideas but constructions, so the literal is generated, not hand
   written.
2. **Shares as data**: put the shares (row 0.10/0.14, word 0.24/0.30,
   selection 0.12, surfaces) next to `pillShare` as named constants, so
   Swift can composite live where a native surface replaces the web window.
3. **xtask**: the design repo's `solve.py` gains the rules above (they are a
   port of `scripts/solve-theme.py`); `xtask` emits them into
   `ThemePalette.swift` and `theme-tokens.json` under `surfaces.*`, `code.*`,
   `heat.*`, `series.*`, and the JSON also exports `selectionWash`.
4. **Tests**: `ThemeContrastTests` gains the floors this script checks (ink
   on every new surface, signal on its row, primary ink on the word highlight,
   cast per fill, heat monotonicity, syntax 4.5:1 on the code surface).
5. **Consumers**: the website's `tokens.generated.css` and gitterrace read
   the JSON; gitterrace then drops its solver for a straight import.
