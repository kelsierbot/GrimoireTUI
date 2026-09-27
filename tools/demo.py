#!/usr/bin/env python3
"""Turn the demo's frames into the moving picture at the top of the README
and the site: an animated WebP and a GIF (for anywhere WebP doesn't move).

    GRIMOIRE_DEMO=/tmp/demo.json cargo test --locked demo -- --ignored
    python3 tools/demo.py /tmp/demo.json assets/readme/demo

Every frame comes from Grimoire's own renderer (src/shots.rs `demo`); this
paints the cells with tools/screenshots.py and strings them together.
"""
import json, os, sys
from PIL import Image

sys.path.insert(0, os.path.dirname(__file__))
from screenshots import BACKGROUNDS, Fonts, paint, window  # noqa: E402

WIDTH = 1200


def main():
    frames = json.load(open(sys.argv[1]))
    out = sys.argv[2]
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    fonts = Fonts()
    images, times = [], []
    for f in frames:
        fr = f["frame"]
        img = window(paint(fr, fonts), BACKGROUNDS.get(fr["theme"], "#121212"),
                     f"grimoire — The Salt Archive · {fr['theme']}")
        img = img.convert("RGB")
        scale = WIDTH / img.width
        img = img.resize((WIDTH, round(img.height * scale)), Image.LANCZOS)
        # Identical neighbours (a frame held twice) become one longer frame.
        if images and img.tobytes() == images[-1].tobytes():
            times[-1] += f["ms"]
            continue
        images.append(img)
        times.append(f["ms"])
    images[0].save(out + ".webp", save_all=True, append_images=images[1:], duration=times,
                   loop=0, quality=82, method=6)
    pal = [im.quantize(colors=128, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE) for im in images]
    pal[0].save(out + ".gif", save_all=True, append_images=pal[1:], duration=times, loop=0,
                optimize=True, disposal=1)
    for ext in ("webp", "gif"):
        print(f"{out}.{ext}: {len(images)} frames, {sum(times) / 1000:.1f}s, "
              f"{os.path.getsize(out + '.' + ext) / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
