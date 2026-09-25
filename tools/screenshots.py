#!/usr/bin/env python3
"""Paint the README's screenshots from frames the real interface drew.

    GRIMOIRE_SHOTS=/tmp/frames.json cargo test --locked shots -- --ignored
    python3 tools/screenshots.py /tmp/frames.json assets/screenshots [exports/]

With an exports folder (the sample book kept by GRIMOIRE_SHOTS_BOOK, then
`grimoire export --docx --pdf --paperback`), it also composes the
manuscript's first pages and a paperback spread from the PDFs.

Every cell comes from Grimoire's own renderer (src/shots.rs); this only turns
cells into pixels: a monospace font with symbol fallbacks, each theme on the
terminal background it was made for, a window frame for the full shots, and
grids of every theme's Pomodoro and Visualizer. Needs Pillow and fontTools.
"""
import json, os, re, subprocess, sys, unicodedata
from PIL import Image, ImageDraw, ImageFilter, ImageFont
from fontTools.ttLib import TTFont

# The terminal background each theme was designed on (Grimoire draws none).
BACKGROUNDS = {
    "Grimoire": "#0d0d10", "Gruvbox Dark": "#282828", "Nord": "#2e3440",
    "Dracula": "#282a36", "Solarized Dark": "#002b36", "Catppuccin Mocha": "#1e1e2e",
    "Tokyo Night": "#1a1b26", "Everforest": "#2d353b", "Lost Forest": "#121a15",
    "Rosé Pine": "#191724", "One Dark": "#282c34", "Monokai": "#272822",
    "Kanagawa": "#1f1f28", "Ayu Mirage": "#1f2430", "Night Owl": "#011627",
    "Material Palenight": "#292d3e", "Synthwave '84": "#262335", "GitHub Dark": "#0d1117",
    "Rainbow": "#15121f",
}
SIZE = 17


def font_file(pattern):
    out = subprocess.run(["fc-match", "-f", "%{file}", pattern], capture_output=True, text=True)
    return out.stdout.strip()


class Fonts:
    def __init__(self):
        regular = font_file("Adwaita Mono:style=Regular") or font_file("monospace")
        self.faces = {
            0: regular,
            1: font_file("Adwaita Mono:style=Bold") or regular,
            2: font_file("Adwaita Mono:style=Italic") or regular,
            3: font_file("Adwaita Mono:style=Bold Italic") or regular,
        }
        self.fallbacks = [font_file(p) for p in ("Noto Sans Mono", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Math", "DejaVu Sans")]
        self.cache, self.cmaps = {}, {}
        probe = self.get(regular)
        self.cw = round(probe.getlength("M"))
        self.ch = round(SIZE * 1.32)

    def get(self, path):
        if path not in self.cache:
            self.cache[path] = ImageFont.truetype(path, SIZE)
        return self.cache[path]

    def cmap(self, path):
        if path not in self.cmaps:
            try:
                self.cmaps[path] = TTFont(path, fontNumber=0, lazy=True).getBestCmap()
            except Exception:
                self.cmaps[path] = {}
        return self.cmaps[path]

    def for_char(self, ch, style):
        face = self.faces[style & 3]
        if ord(ch) in self.cmap(face):
            return self.get(face)
        for path in self.fallbacks:
            if path and ord(ch) in self.cmap(path):
                return self.get(path)
        return self.get(face)


# Box drawing, drawn as strokes so lines join across rows the way a terminal
# draws them: (up, down, left, right) arms, 1 light / 2 heavy / 3 double.
BOX = {
    "─": (0, 0, 1, 1), "│": (1, 1, 0, 0), "━": (0, 0, 2, 2), "┃": (2, 2, 0, 0),
    "┌": (0, 1, 0, 1), "┐": (0, 1, 1, 0), "└": (1, 0, 0, 1), "┘": (1, 0, 1, 0),
    "├": (1, 1, 0, 1), "┤": (1, 1, 1, 0), "┬": (0, 1, 1, 1), "┴": (1, 0, 1, 1), "┼": (1, 1, 1, 1),
    "═": (0, 0, 3, 3), "║": (3, 3, 0, 0), "╔": (0, 3, 0, 3), "╗": (0, 3, 3, 0), "╚": (3, 0, 0, 3), "╝": (3, 0, 3, 0),
}
ROUND = {"╭": (0, 1, 0, 1), "╮": (0, 1, 1, 0), "╰": (1, 0, 0, 1), "╯": (1, 0, 1, 0)}


def blend(a, b, t):
    a, b = a.lstrip("#"), b.lstrip("#")
    ca = [int(a[i:i + 2], 16) for i in (0, 2, 4)]
    cb = [int(b[i:i + 2], 16) for i in (0, 2, 4)]
    return "#" + "".join(f"{round(x + (y - x) * t):02x}" for x, y in zip(ca, cb))


def block(d, ch, x, y, w, h, fill, bg):
    """Block elements fill their cell exactly, as a terminal draws them."""
    o = ord(ch)
    if ch in "░▒▓":
        d.rectangle([x, y, x + w - 1, y + h - 1], fill=blend(bg, fill, {"░": 0.25, "▒": 0.5, "▓": 0.75}[ch]))
    elif ch == "█":
        d.rectangle([x, y, x + w - 1, y + h - 1], fill=fill)
    elif ch == "▀":
        d.rectangle([x, y, x + w - 1, y + h // 2 - 1], fill=fill)
    elif 0x2581 <= o <= 0x2587:  # ▁▂▃▄▅▆▇: lower eighths
        n = o - 0x2580
        d.rectangle([x, y + h - round(h * n / 8), x + w - 1, y + h - 1], fill=fill)
    elif ch == "▔":
        d.rectangle([x, y, x + w - 1, y + max(1, h // 8) - 1], fill=fill)
    elif ch == "▌":
        d.rectangle([x, y, x + w // 2 - 1, y + h - 1], fill=fill)
    elif ch == "▐":
        d.rectangle([x + w // 2, y, x + w - 1, y + h - 1], fill=fill)
    else:
        return False
    return True


def box(d, ch, x, y, w, h, fill):
    arms = BOX.get(ch) or ROUND.get(ch)
    if not arms:
        return False
    cx, cy = x + w // 2, y + h // 2
    up, down, left, right = arms
    if ch in ROUND:
        r = min(w, h) // 2
        # the straight arms, then a quarter circle joining them
        if up: d.line([cx, y, cx, cy - r], fill=fill)
        if down: d.line([cx, cy + r, cx, y + h], fill=fill)
        if left: d.line([x, cy, cx - r, cy], fill=fill)
        if right: d.line([cx + r, cy, x + w, cy], fill=fill)
        bx = cx - r if right else cx - r * 2 if left else cx - r
        by = cy - r if down else cy - r * 2 if up else cy - r
        start = {(0, 1, 0, 1): 180, (0, 1, 1, 0): 270, (1, 0, 0, 1): 90, (1, 0, 1, 0): 0}[arms]
        ox = cx if right else cx - 2 * r
        oy = cy if down else cy - 2 * r
        d.arc([ox, oy, ox + 2 * r, oy + 2 * r], start, start + 90, fill=fill)
        return True
    for n, (x0, y0, x1, y1) in zip(arms, [(cx, y, cx, cy), (cx, cy, cx, y + h), (x, cy, cx, cy), (cx, cy, x + w, cy)]):
        if n == 1:
            d.line([x0, y0, x1, y1], fill=fill)
        elif n == 2:
            d.line([x0, y0, x1, y1], fill=fill, width=3)
        elif n == 3:
            if x0 == x1:
                d.line([x0 - 2, y0, x1 - 2, y1], fill=fill); d.line([x0 + 2, y0, x1 + 2, y1], fill=fill)
            else:
                d.line([x0, y0 - 2, x1, y1 - 2], fill=fill); d.line([x0, y0 + 2, x1, y1 + 2], fill=fill)
    return True


def paint(frame, fonts, crop=None):
    x0, y0, w, h = crop or (0, 0, frame["w"], frame["h"])
    bg = BACKGROUNDS.get(frame["theme"], "#121212")
    img = Image.new("RGB", (w * fonts.cw, h * fonts.ch), bg)
    d = ImageDraw.Draw(img)
    for y in range(h):
        row = frame["rows"][y0 + y]
        for x in range(w):
            sym, fg, cbg, style = row[x0 + x]
            px, py = x * fonts.cw, y * fonts.ch
            if cbg:
                d.rectangle([px, py, px + fonts.cw, py + fonts.ch], fill=cbg)
            if sym.strip() and (block(d, sym[0], px, py, fonts.cw, fonts.ch, fg or "#cccccc", cbg or bg)
                                or box(d, sym[0], px, py, fonts.cw, fonts.ch, fg or "#cccccc")):
                pass
            elif sym.strip():
                f = fonts.for_char(sym[0], style)
                d.text((px, py + 2), sym, font=f, fill=fg or "#cccccc")
            if style & 4:
                d.line([px, py + fonts.ch - 3, px + fonts.cw, py + fonts.ch - 3], fill=fg or "#cccccc")
    return img


def window(img, bg, title):
    """A quiet window frame: title bar, rounded corners, a soft shadow."""
    pad, bar, r = 48, 34, 12
    w, h = img.width + 24, img.height + bar + 16
    out = Image.new("RGB", (w + 2 * pad, h + 2 * pad), "#0b0b0e")
    grad = ImageDraw.Draw(out)
    for y in range(out.height):  # a faint vertical wash behind the window
        t = y / out.height
        c = tuple(int(a + (b - a) * t) for a, b in zip((22, 20, 30), (8, 8, 11)))
        grad.line([(0, y), (out.width, y)], fill=c)
    shadow = Image.new("L", out.size, 0)
    ImageDraw.Draw(shadow).rounded_rectangle([pad + 6, pad + 14, pad + w + 6, pad + h + 14], r, fill=170)
    shadow = shadow.filter(ImageFilter.GaussianBlur(18))
    out.paste(Image.new("RGB", out.size, "#000000"), (0, 0), shadow)
    frame = Image.new("RGB", (w, h), bg)
    fd = ImageDraw.Draw(frame)
    fd.rectangle([0, 0, w, bar], fill=tuple(max(0, int(c * 0.8)) for c in Image.new("RGB", (1, 1), bg).getpixel((0, 0))))
    for i, c in enumerate(("#ff5f57", "#febc2e", "#28c840")):
        fd.ellipse([16 + i * 22, 11, 28 + i * 22, 23], fill=c)
    tf = ImageFont.truetype(font_file("Adwaita Mono:style=Regular") or font_file("monospace"), 14)
    tw = fd.textlength(title, font=tf)
    fd.text(((w - tw) / 2, 9), title, font=tf, fill="#9a9aa6")
    frame.paste(img, (12, bar + 6))
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, w - 1, h - 1], r, fill=255)
    out.paste(frame, (pad, pad), mask)
    return out


def grid(tiles, fonts, cols, title):
    """Tiles of (label, image) on a dark board, labels under each."""
    tw = max(t.width for _, t in tiles)
    th = max(t.height for _, t in tiles)
    gap, label = 22, 30
    rows = (len(tiles) + cols - 1) // cols
    head = 0
    out = Image.new("RGB", (cols * tw + (cols + 1) * gap, head + rows * (th + label) + (rows + 1) * gap), "#0b0b0e")
    d = ImageDraw.Draw(out)
    lf = ImageFont.truetype(font_file("Adwaita Sans") or font_file("sans"), 15)
    for i, (name, t) in enumerate(tiles):
        r, c = divmod(i, cols)
        x = gap + c * (tw + gap)
        y = head + gap + r * (th + label + gap)
        out.paste(t, (x, y))
        lw = d.textlength(name, font=lf)
        d.text((x + (tw - lw) / 2, y + th + 6), name, font=lf, fill="#a8a8b4")
    return out


def backdrop(w, h):
    """The dark wash every composed image sits on."""
    out = Image.new("RGB", (w, h), "#0b0b0e")
    d = ImageDraw.Draw(out)
    for y in range(h):
        t = y / h
        d.line([(0, y), (w, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip((22, 20, 30), (8, 8, 11))))
    return out


def shadowed(canvas, img, x, y, r=18):
    shadow = Image.new("L", canvas.size, 0)
    ImageDraw.Draw(shadow).rectangle([x + 6, y + 12, x + img.width + 6, y + img.height + 12], fill=160)
    canvas.paste(Image.new("RGB", canvas.size, "#000000"), (0, 0), shadow.filter(ImageFilter.GaussianBlur(r)))
    canvas.paste(img, (x, y))


def render_pdf(pdf, dpi):
    """Every page of `pdf` as an image (pdftoppm)."""
    import glob, tempfile
    tmp = tempfile.mkdtemp()
    subprocess.run(["pdftoppm", "-png", "-r", str(dpi), pdf, os.path.join(tmp, "p")], check=True)
    return [Image.open(p).convert("RGB") for p in sorted(glob.glob(os.path.join(tmp, "p-*.png")))]


def ink(img):
    g = img.convert("L").resize((80, 120))
    return sum(1 for v in g.tobytes() if v < 200) / (80 * 120)


def pages(exports, out):
    """The manuscript's first three pages, and a paperback spread."""
    import glob
    ms = sorted(glob.glob(os.path.join(exports, "*_Manuscript*.pdf")))
    pb = sorted(glob.glob(os.path.join(exports, "*_Paperback*.pdf")))
    if ms:
        pgs = render_pdf(ms[0], 110)[:3]
        w, h = pgs[0].size
        gap, pad = 40, 60
        c = backdrop(len(pgs) * w + (len(pgs) - 1) * gap + 2 * pad, h + 2 * pad)
        for i, p in enumerate(pgs):
            shadowed(c, p, pad + i * (w + gap), pad)
        c.save(os.path.join(out, "manuscript.png"), optimize=True)
        print("manuscript")
    if pb:
        pgs = render_pdf(pb[0], 130)
        # The last left/right pair with words on both: a verso facing a recto.
        pairs = [(i, i + 1) for i in range(1, len(pgs) - 1, 2) if ink(pgs[i]) > 0.004 and ink(pgs[i + 1]) > 0.004]
        if pairs:
            a, b = pairs[-1]
            w, h = pgs[a].size
            pad = 70
            c = backdrop(2 * w + 2 * pad, h + 2 * pad)
            spread = Image.new("RGB", (2 * w, h), "#ffffff")
            spread.paste(pgs[a], (0, 0))
            spread.paste(pgs[b], (w, 0))
            # A soft gutter where the pages meet.
            g = ImageDraw.Draw(spread)
            for dx in range(18):
                shade = int(255 - (18 - dx) * 2.2)
                g.line([(w - dx, 0), (w - dx, h)], fill=(shade, shade, shade))
                g.line([(w + dx, 0), (w + dx, h)], fill=(shade, shade, shade))
            shadowed(c, spread, pad, pad)
            c.save(os.path.join(out, "paperback.png"), optimize=True)
            print("paperback")


def slug(name):
    """A file name for a theme: "Rosé Pine" → rose-pine, "Synthwave '84" → synthwave-84."""
    flat = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode()
    return re.sub(r"[^a-z0-9]+", "-", flat.lower()).strip("-")


def main():
    frames = json.load(open(sys.argv[1]))
    out = sys.argv[2]
    os.makedirs(out, exist_ok=True)
    fonts = Fonts()
    pomos, vizes = [], []
    for fr in frames:
        name = fr["name"]
        if name.startswith("theme/"):
            # The desk in each theme, and that theme's Pomodoro on its own
            # (the scene inside its pane's frame), for the theme switchers.
            theme = name.split("/", 1)[1]
            os.makedirs(os.path.join(out, "themes"), exist_ok=True)
            img = window(paint(fr, fonts), BACKGROUNDS.get(theme, "#121212"), f"grimoire — The Salt Archive · {theme}")
            img.save(os.path.join(out, "themes", f"{slug(theme)}.png"), optimize=True)
            print(name)
            continue
        if "/" in name:
            kind, theme = name.split("/", 1)
            tile = paint(fr, fonts, fr["crop"])
            (pomos if kind == "pomodoro" else vizes).append((theme, tile))
            if kind == "pomodoro":
                os.makedirs(os.path.join(out, "themes"), exist_ok=True)
                inner = tile.crop((fonts.cw, fonts.ch, tile.width - fonts.cw, tile.height - fonts.ch))
                inner.save(os.path.join(out, "themes", f"pomodoro-{slug(theme)}.png"), optimize=True)
            continue
        img = window(paint(fr, fonts), BACKGROUNDS.get(fr["theme"], "#121212"), f"grimoire — The Salt Archive · {fr['theme']}")
        img.save(os.path.join(out, f"{name}.png"), optimize=True)
        print(name)
    if pomos:
        grid(pomos, fonts, 5, "Pomodoro").save(os.path.join(out, "pomodoros.png"), optimize=True)
        print("pomodoros")
    if vizes:
        grid(vizes, fonts, 5, "Visualizer").save(os.path.join(out, "visualizers.png"), optimize=True)
        print("visualizers")
    if len(sys.argv) > 3:
        pages(sys.argv[3], out)


if __name__ == "__main__":
    main()
