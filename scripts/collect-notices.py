#!/usr/bin/env python3
"""Collect license files from Cargo's resolved target sources for packaging."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import re

p = argparse.ArgumentParser()
p.add_argument("--target", default="x86_64-pc-windows-gnu")
p.add_argument("--out", default="dist/third-party-notices")
a = p.parse_args()
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", a.target]))
ids = {n["id"] for n in metadata["resolve"]["nodes"]}
tree = subprocess.check_output(["cargo", "tree", "--locked", "--target", a.target, "--edges", "normal,build", "--prefix", "none", "--format", "{p}"], text=True)
active = set(re.findall(r"^([\w-]+) v([^\s]+)", tree, re.MULTILINE))
out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
inventory = []
for package in metadata["packages"]:
    if package["id"] not in ids or package["name"] == "shuffle-room" or (package["name"],package["version"]) not in active: continue
    root = Path(package["manifest_path"]).parent
    dest = out / f'{package["name"]}-{package["version"]}'
    matches = []
    for file in root.rglob("*"):
        if file.is_file() and (file.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")) or file.name.upper() in ("OFL.TXT", "OFL")):
            relative = file.relative_to(root)
            (dest / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(file, dest / relative)
            matches.append(str(relative))
    if package.get("license_file"):
        file = root / package["license_file"]
        if file.is_file() and package["license_file"] not in matches:
            dest.mkdir(parents=True, exist_ok=True); shutil.copyfile(file, dest / file.name); matches.append(file.name)
    overrides = Path(__file__).resolve().parent.parent / "packaging" / "notice-overrides" / package["name"]
    if overrides.is_dir():
        for file in overrides.iterdir():
            if file.is_file():
                dest.mkdir(parents=True, exist_ok=True); shutil.copyfile(file, dest / file.name); matches.append(file.name)
    inventory.append({"name":package["name"], "version":package["version"], "license":package.get("license"), "repository":package.get("repository"), "notice_files":matches})
(out / "dependencies.json").write_text(json.dumps(inventory, indent=2) + "\n")
missing = [f'{p["name"]} {p["version"]}: {p["license"]}' for p in inventory if not p["notice_files"]]
print(f"Collected notices for {len(inventory)} crates in {out}")
if missing: print("Review packages without included license texts:\n" + "\n".join(missing))
