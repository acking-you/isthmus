# Bundled fonts

`GoogleSansFlex-*.ttf` (prose) and `GeistMono-*.ttf` (key labels and bundle
ids) are the design system's faces, shared with POCOX and pb-mapper, under the
SIL Open Font License 1.1: see `OFL-GoogleSansFlex.txt` and
`OFL-GeistMono.txt`. `lib/main.dart` registers both texts with the licence
registry, because the OFL asks that the notice travel with the font.

They are static instances, one per weight, because Flutter maps `fontWeight`
to the nearest declared face and never onto a `wght` axis.

No Chinese face is bundled. Isthmus runs on macOS only, where PingFang SC
ships in every weight the theme asks for; `lib/src/theme/fonts.dart` names it
first in the fallback chain.
