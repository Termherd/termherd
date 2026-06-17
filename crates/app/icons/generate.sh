#!/usr/bin/env bash
# Regenerate every raster icon from the single source of truth, icon.svg.
#
# macOS-only (uses iconutil for the .icns). Requires rsvg-convert and
# ImageMagick (`magick`). Run from this directory:
#
#     ./generate.sh
#
# Outputs: icon.png (1024), sized PNGs, icon.ico (Windows), icon.icns (macOS).
set -euo pipefail
cd "$(dirname "$0")"

command -v rsvg-convert >/dev/null || { echo "need rsvg-convert" >&2; exit 1; }
command -v magick       >/dev/null || { echo "need ImageMagick (magick)" >&2; exit 1; }
command -v iconutil     >/dev/null || { echo "need iconutil (macOS)" >&2; exit 1; }

# Master + Linux desktop sizes.
rsvg-convert -w 1024 -h 1024 icon.svg | magick - -strip icon.png
for s in 32 128 256 512; do
  rsvg-convert -w "$s" -h "$s" icon.svg -o "${s}x${s}.png"
done
cp 512x512.png 128x128@2x.png

# Windows multi-resolution .ico.
magick icon.png -define icon:auto-resize=16,24,32,48,64,128,256 icon.ico

# macOS .icns via the canonical iconset → iconutil pipeline.
rm -rf icon.iconset && mkdir icon.iconset
for pair in 16:icon_16x16 32:icon_16x16@2x 32:icon_32x32 64:icon_32x32@2x \
            128:icon_128x128 256:icon_128x128@2x 256:icon_256x256 \
            512:icon_256x256@2x 512:icon_512x512 1024:icon_512x512@2x; do
  s="${pair%%:*}"; name="${pair##*:}"
  rsvg-convert -w "$s" -h "$s" icon.svg -o "icon.iconset/${name}.png"
done
iconutil -c icns icon.iconset -o icon.icns
rm -rf icon.iconset

echo "Regenerated: icon.png icon.ico icon.icns + sized PNGs"
