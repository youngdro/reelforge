# Bundled fonts

`NotoSansSC-Regular.subset.ttf` and `NotoSansSC-Bold.subset.ttf` are Noto Sans SC 2.004 (the static
Regular and Bold instances Google Fonts distributes), subset to every character of GB 2312 (6,763
Hanzi and its symbols), Basic Latin, Latin-1 and Latin Extended-A, general punctuation, letterlike
symbols, CJK symbols and punctuation, and full-width forms (7,883 characters), with all OpenType
layout features kept and hinting removed. Licensed under the SIL Open Font License 1.1
(`OFL-NotoSansSC.txt`); the subset is a Modified Version under the OFL, and the reserved font
name ("Source") is not used.

Made with fontTools 4.68:

```sh
pyftsubset NotoSansSC_700Bold.ttf --text-file=gb2312.txt --layout-features='*' --no-hinting \
  --name-IDs='*' --name-languages='*' --output-file=NotoSansSC-Bold.subset.ttf
```

Characters outside the subset fall back to the other faces the renderer knows (the bundled Latin
faces, then any `font` asset a spec brings). A spec that needs rarer characters ships its own font as
a `font` asset, which also keeps renders identical on every machine.
