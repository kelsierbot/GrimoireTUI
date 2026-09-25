#!/usr/bin/env python3
"""The themes, for the website's theme switcher: every preset's colours
(from src/theme.rs, in picker order) and its terminal background (from
tools/screenshots.py), written to site/src/themes.json.

    python3 tools/site_themes.py
"""
import json, os, re, unicodedata

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIELDS = ["accent", "text", "dim", "border", "sel", "warn", "sun", "moon", "foliage", "bark", "bloom", "turf"]


def slug(name):
    flat = unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode()
    return re.sub(r"[^a-z0-9]+", "-", flat.lower()).strip("-")


def main():
    src = open(os.path.join(ROOT, "src/theme.rs"), encoding="utf-8").read()
    body = src[src.index("pub fn presets()"):]
    body = body[: body.index("];")]
    consts = dict(re.findall(r'pub const (\w+): &str = "([^"]+)";', src))
    shots = open(os.path.join(ROOT, "tools/screenshots.py"), encoding="utf-8").read()
    bgs = dict(re.findall(r'"([^"]+)": "(#[0-9a-fA-F]{6})"', shots[shots.index("BACKGROUNDS"):shots.index("}", shots.index("BACKGROUNDS"))]))
    themes = []
    for name, args in re.findall(r'\(\s*("[^"]+"|[A-Z_]+),\s*theme!\(\s*"[^"]*",([^)]*)\)', body):
        name = name.strip('"') if name.startswith('"') else consts[name]
        hexes = re.findall(r"0x([0-9a-fA-F]{6})", args)
        assert len(hexes) == len(FIELDS), name
        t = {"name": name, "slug": slug(name), "bg": bgs[name]}
        t.update({f: "#" + h.lower() for f, h in zip(FIELDS, hexes)})
        themes.append(t)
    assert len(themes) == 19, len(themes)
    out = os.path.join(ROOT, "site/src/themes.json")
    json.dump(themes, open(out, "w"), indent=1, ensure_ascii=False)
    print(f"{len(themes)} themes → {os.path.relpath(out, ROOT)}")


if __name__ == "__main__":
    main()
