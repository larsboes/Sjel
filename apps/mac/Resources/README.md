# Resources

`MenuBarIcon.pdf` is `brand/sjel-hedgehog.svg` with the background rect removed and the mark
filled black, so macOS can tint it as a template image. Regenerate it after the mark changes:

```sh
sed -e 's/<rect[^>]*\/>//' -e 's/#F7F7F3/#000000/' brand/sjel-hedgehog.svg \
  | rsvg-convert -f pdf -h 36 -o apps/mac/Resources/MenuBarIcon.pdf
```
