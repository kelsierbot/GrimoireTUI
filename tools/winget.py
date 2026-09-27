#!/usr/bin/env python3
"""Send a Grimoire release to winget, the Windows Package Manager.

    tools/winget.py 0.6.0             open the pull request on winget-pkgs
    tools/winget.py 0.6.0 --dry-run   print the manifests, send nothing

Reads the release's Windows zip and its .sha256 from GitHub, writes the three
manifests winget wants, and opens a pull request on microsoft/winget-pkgs from
the account's fork of it (made on first use). Uses the GitHub token in
~/.git-credentials; nothing is stored anywhere else.

Part of every release, after the Release workflow has published the zip. The
first version is reviewed by a person at Microsoft; after that their bots
usually merge each new one within a day. Then `winget install
CatfinityStudios.Grimoire` (or `winget upgrade`) gets it.
"""
import json, re, sys, time, urllib.request
from pathlib import Path

REPO = "kelsierbot/GrimoireTUI"
UPSTREAM = "microsoft/winget-pkgs"
ID = "CatfinityStudios.Grimoire"
SCHEMA = "1.12.0"
ZIP = "grimoire-tui-x86_64-pc-windows-msvc.zip"


def token():
    for line in (Path.home() / ".git-credentials").read_text().splitlines():
        m = re.match(r"https://[^:]+:([^@]+)@github\.com", line)
        if m:
            return m.group(1)
    sys.exit("no GitHub token in ~/.git-credentials")


def api(method, path, body=None, tok=None):
    req = urllib.request.Request(
        f"https://api.github.com{path}",
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={"Authorization": f"token {tok}", "Accept": "application/vnd.github+json"},
    )
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        if e.code == 404 and method == "GET":
            return None
        sys.exit(f"{method} {path}: {e.code} {e.read().decode()[:400]}")


def manifests(version, sha):
    url = f"https://github.com/{REPO}/releases/download/v{version}/{ZIP}"
    head = lambda kind: f"# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.{SCHEMA}.schema.json\n\n"
    return {
        f"{ID}.yaml": head("version") + f"""PackageIdentifier: {ID}
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: {SCHEMA}
""",
        f"{ID}.installer.yaml": head("installer") + f"""PackageIdentifier: {ID}
PackageVersion: {version}
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
- RelativeFilePath: grimoire.exe
  PortableCommandAlias: grimoire
Installers:
- Architecture: x64
  InstallerUrl: {url}
  InstallerSha256: {sha.upper()}
ManifestType: installer
ManifestVersion: {SCHEMA}
""",
        f"{ID}.locale.en-US.yaml": head("defaultLocale") + f"""PackageIdentifier: {ID}
PackageVersion: {version}
PackageLocale: en-US
Publisher: Catfinity Studios
PublisherUrl: https://catfinity.com
PublisherSupportUrl: https://github.com/{REPO}/issues
Author: Catfinity Studios
PackageName: Grimoire
PackageUrl: https://grimoiretui.com
License: MIT
LicenseUrl: https://github.com/{REPO}/blob/main/LICENSE
Copyright: Copyright (c) Josh King, Catfinity Studios
ShortDescription: A cozy writing desk for novels that lives in your terminal.
Description: |-
  Grimoire is a writing desk for novels that lives in your terminal. Your book is
  an outline of parts, chapters and scenes beside a quiet page to write on, with
  notes and TKs, spelling, sprints, a Pomodoro, music and nineteen themes. When
  it's ready, export it to Word, PDF, EPUB or a paperback. Every book is a folder
  of plain Markdown files, so your words are always yours. Free, open source, and
  made by Catfinity Studios, makers of Catfinity.
Moniker: grimoire
Tags:
- writing
- novel
- manuscript
- markdown
- terminal
- tui
- editor
- epub
- docx
ReleaseNotesUrl: https://github.com/{REPO}/releases/tag/v{version}
ManifestType: defaultLocale
ManifestVersion: {SCHEMA}
""",
    }


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) != 1:
        sys.exit(__doc__)
    version = args[0].lstrip("v")
    dry = "--dry-run" in sys.argv
    sha_url = f"https://github.com/{REPO}/releases/download/v{version}/{ZIP}.sha256"
    with urllib.request.urlopen(sha_url) as r:
        sha = r.read().decode().split()[0]
    files = manifests(version, sha)
    if dry:
        for name, body in files.items():
            print(f"--- {name}\n{body}")
        return

    tok = token()
    me = api("GET", "/user", tok=tok)["login"]
    if api("GET", f"/repos/{me}/winget-pkgs", tok=tok) is None:
        api("POST", f"/repos/{UPSTREAM}/forks", {"default_branch_only": True}, tok=tok)
        for _ in range(60):
            if api("GET", f"/repos/{me}/winget-pkgs", tok=tok):
                break
            time.sleep(5)
    base = api("GET", f"/repos/{UPSTREAM}/git/ref/heads/master", tok=tok)["object"]["sha"]
    base_tree = api("GET", f"/repos/{UPSTREAM}/git/commits/{base}", tok=tok)["tree"]["sha"]
    folder = f"manifests/c/CatfinityStudios/Grimoire/{version}"
    tree = api("POST", f"/repos/{me}/winget-pkgs/git/trees", {
        "base_tree": base_tree,
        "tree": [{"path": f"{folder}/{n}", "mode": "100644", "type": "blob", "content": b}
                 for n, b in files.items()],
    }, tok=tok)["sha"]
    commit = api("POST", f"/repos/{me}/winget-pkgs/git/commits", {
        "message": f"New version: {ID} version {version}", "tree": tree, "parents": [base],
    }, tok=tok)["sha"]
    branch = f"{ID}-{version}"
    ref = f"/repos/{me}/winget-pkgs/git/refs"
    if api("GET", f"{ref}/heads/{branch}", tok=tok):
        api("PATCH", f"{ref}/heads/{branch}", {"sha": commit, "force": True}, tok=tok)
    else:
        api("POST", ref, {"ref": f"refs/heads/{branch}", "sha": commit}, tok=tok)
    open_prs = api("GET", f"/repos/{UPSTREAM}/pulls?head={me}:{branch}&state=open", tok=tok)
    if open_prs:
        print(f"updated {open_prs[0]['html_url']}")
        return
    first = api("GET", f"/repos/{UPSTREAM}/contents/manifests/c/CatfinityStudios/Grimoire", tok=tok) is None
    pr = api("POST", f"/repos/{UPSTREAM}/pulls", {
        "title": f"{'New package' if first else 'New version'}: {ID} version {version}",
        "head": f"{me}:{branch}",
        "base": "master",
        "body": (
            f"{'Adds' if first else 'Updates'} **Grimoire** ({ID}) {version}, a terminal writing desk for novels "
            f"from Catfinity Studios. Portable zip from the project's GitHub release; the exe is statically "
            f"linked, so it has no dependencies.\n\n"
            f"- Release: https://github.com/{REPO}/releases/tag/v{version}\n"
            f"- Home: https://grimoiretui.com\n"
        ),
    }, tok=tok)
    print(pr["html_url"])


if __name__ == "__main__":
    main()
