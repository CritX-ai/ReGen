#!/usr/bin/env python3
"""Project the animated SVG's default artwork into an animation-free SVG."""

import argparse
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
SVG = "{http://www.w3.org/2000/svg}"


def projection(source):
    root = ET.fromstring(source)
    # Every visual default is an SVG attribute. CSS supplies motion only.
    for parent in root.iter():
        for child in list(parent):
            if child.tag == SVG + "style":
                parent.remove(child)
    ET.register_namespace("", "http://www.w3.org/2000/svg")
    ET.indent(root, space="  ")
    return ET.tostring(root, encoding="unicode") + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Fail instead of writing if the static projection has drifted")
    args = parser.parse_args()
    assets = ROOT / "site" / "assets"
    expected = projection((assets / "regen-logo.svg").read_text(encoding="utf-8"))
    destination = assets / "regen-logo-static.svg"
    if args.check:
        if destination.read_text(encoding="utf-8") != expected:
            raise SystemExit("Static logo differs; run python3 scripts/project_logo.py")
        print("Static logo matches the animated SVG's default artwork")
    else:
        destination.write_text(expected, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
