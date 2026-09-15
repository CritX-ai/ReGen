#!/usr/bin/env python3
"""Combine LLVM executions of the same source region; used by CI and pre-commit."""

import argparse
import json
from pathlib import Path


def project(report, expected):
    data = report["data"][0]
    sources = [entry["filename"] for entry in data["files"]]
    if sorted(sources) != sorted(expected):
        raise RuntimeError("production Rust files missing from coverage")
    regions = {}
    for function in data["functions"]:
        for region in function["regions"]:
            filename = function["filenames"][region[5]]
            if region[7] != 0 or filename not in sources:
                continue
            key = (filename, tuple(region[:4]))
            regions[key] = regions.get(key, False) or region[4] > 0

    def summary(hits):
        return {"count": len(hits), "covered": sum(hits)}

    return {
        "metric": "Executable source regions (union across instantiations)",
        "totals": summary(list(regions.values())),
        "files": [dict(summary([hit for (filename, _), hit in regions.items() if filename == source]), filename=source) for source in sources],
        "uncovered": [{"filename": filename, "span": list(span)} for (filename, span), hit in sorted(regions.items()) if not hit],
    }


def require_full(result):
    if not result["totals"]["count"] or result["totals"]["covered"] != result["totals"]["count"] or result["uncovered"]:
        raise RuntimeError("uncovered production Rust source regions")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, default=Path("target/coverage/rust.json"))
    parser.add_argument("--output", type=Path, default=Path("target/coverage/rust-source-regions.json"))
    parser.add_argument("--require-full", action="store_true")
    args = parser.parse_args()
    source = Path(__file__).resolve().parents[1] / "src"
    result = project(json.loads(args.report.read_text()), [str(path.resolve()) for path in source.rglob("*.rs")])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(f"Rust source regions: {result['totals']['covered']}/{result['totals']['count']}", flush=True)
    if args.require_full:
        require_full(result)


if __name__ == "__main__":
    main()
