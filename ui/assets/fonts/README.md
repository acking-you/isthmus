# Bundled fonts

`GoogleSansFlex-*.ttf` (prose) and `GeistMono-*.ttf` (key labels and bundle
ids) are the design system's faces, shared with POCOX and pb-mapper, under the
SIL Open Font License 1.1: see `OFL-GoogleSansFlex.txt` and
`OFL-GeistMono.txt`. `lib/main.dart` registers both texts with the licence
registry, because the OFL asks that the notice travel with the font.

They are static instances, one per weight, because Flutter maps `fontWeight`
to the nearest declared face and never onto a `wght` axis.

## Chinese on Windows

`NotoSansSC-*.subset.ttf` are subsets of **Noto Sans SC** under the SIL Open
Font License 1.1 (`OFL-NotoSansSC.txt`, verbatim from
<https://github.com/notofonts/noto-cjk>), the same files pb-mapper ships.
Windows' own Chinese face, Microsoft YaHei, has Light, Regular and Bold and
nothing between: a run asking for 500 comes back Regular beside Segoe UI
Semibold, and the line renders at two weights. `lib/src/theme/fonts.dart`
puts the subset first in Windows' fallback chain; a Mac keeps PingFang SC,
which has six real weights.

They cover everything in GB2312 (6763 hanzi), ASCII, CJK and fullwidth
punctuation: about 7000 characters, 2.1 MB per weight, four weights (400 and
500 for Material's own text, 600 and 700 for headings). Every character in
`lib/l10n/app_zh.arb` but `›`, which Segoe UI draws, is in them. Static
instances, since Flutter never maps `fontWeight` onto a `wght` axis.

To regenerate (with this app's strings added to the set):

```
cd ui/tool
npm install subset-font iconv-lite
node gen_cjk_subset.mjs <path-to-NotoSansSC-VF.ttf>
```

The 17 MB variable source is not checked in; get it from the noto-cjk
releases.
