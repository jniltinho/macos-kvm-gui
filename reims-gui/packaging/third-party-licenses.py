#!/usr/bin/env python3
"""THIRD_PARTY_LICENSES for the crates compiled into `reims-gui`, from `cargo metadata` JSON.

Usage: third-party-licenses.py <metadata.json>  (prints to stdout)
Runs where the crate sources are (the build container after a build, or a `cargo vendor` tree):
each crate's license files are copied in full. Only normal dependencies reachable from the
`reims-gui` package count (build scripts and proc macros run at build time only, but proc-macro
crates are normal dependencies in metadata and are listed too, which errs on the safe side).
"""
import json
import os
import re
import sys

meta = json.load(open(sys.argv[1]))
packages = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = next(p["id"] for p in meta["packages"] if p["name"] == "reims-gui")

seen, todo = set(), [root]
while todo:
    for dep in nodes[todo.pop()]["deps"]:
        if any(k["kind"] is None for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
            seen.add(dep["pkg"])
            todo.append(dep["pkg"])

crates = sorted((packages[i] for i in seen if packages[i]["source"]), key=lambda p: (p["name"], p["version"]))
LICENSE_FILE = re.compile(r"^(licen[sc]e|copying|notice|unlicense|copyright)", re.I)
# Crates whose package ships no license file: the text from their repository, in licenses/.
KNOWN_TEXTS = {"include_dir": "include_dir-MIT.txt", "include_dir_macros": "include_dir-MIT.txt"}


def license_files(pkg):
    top = os.path.dirname(pkg["manifest_path"])
    found = []
    for sub in ("", "LICENSES", "license", "licenses", "fonts"):
        d = os.path.join(top, sub)
        if os.path.isdir(d):
            # Top level: license-named files; license folders: everything; fonts/: the .txt licenses.
            found += [os.path.join(d, f) for f in os.listdir(d) if os.path.isfile(os.path.join(d, f))
                      and (LICENSE_FILE.match(f) if sub == "" else f.endswith(".txt") if sub == "fonts" else True)]
    if pkg.get("license_file"):
        found.append(os.path.join(top, pkg["license_file"]))
    if not found and pkg["name"] in KNOWN_TEXTS:
        found.append(os.path.join(os.path.dirname(__file__), "licenses", KNOWN_TEXTS[pkg["name"]]))
    return sorted(set(found))


missing = [p["name"] for p in crates if not p.get("license") and not p.get("license_file")]
if missing:
    sys.exit("crates without a license: " + ", ".join(missing))

out = sys.stdout
out.write("Third-party Rust crates compiled into reims-gui\n")
out.write("Generated from `cargo metadata` (Cargo.lock) and each crate's own license files.\n")
out.write("The reims-gui binary is distributed under GPL-3.0-or-later.\n\n")
for p in crates:
    out.write(f"{p['name']} {p['version']}  {p.get('license') or 'see file'}  {p.get('repository') or ''}".rstrip() + "\n")
no_text = []
for p in crates:
    files = license_files(p)
    if not files:
        no_text.append(p)
        continue
    out.write("\n" + "=" * 78 + f"\n{p['name']} {p['version']} ({p.get('license') or 'see file'})\n" + "=" * 78 + "\n")
    for f in files:
        out.write(f"\n--- {os.path.basename(f)} ---\n\n")
        out.write(open(f, encoding="utf-8", errors="replace").read().rstrip() + "\n")
if no_text:
    # These crates ship no license file. All of them offer Apache-2.0, whose text needs no
    # copyright line, so they are used under it; a crate without that option stops the build.
    other = [p["name"] for p in no_text if "Apache-2.0" not in (p.get("license") or "")]
    if other:
        sys.exit("crates without a license file nor an Apache-2.0 option: " + ", ".join(other))
    out.write("\n" + "=" * 78 + "\nCrates that ship no license file, used under Apache-2.0 (text below)\n"
              + "=" * 78 + "\n\n")
    for p in no_text:
        out.write(f"{p['name']} {p['version']}  {p['license']}\n")
    out.write("\n" + open(os.path.join(os.path.dirname(__file__), "licenses", "Apache-2.0.txt")).read())
print(f"{len(crates)} crates, {len(no_text)} without a license file", file=sys.stderr)
