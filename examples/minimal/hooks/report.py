#!/usr/bin/env python3
"""Opt-in example: report the build phase without modifying generated output."""

import json
import os

FIELDS = (
    "REGEN_SITE", "REGEN_OUTPUT", "REGEN_PROFILE", "REGEN_STATUS",
    "REGEN_ERROR", "REGEN_ERROR_TRUNCATED",
)

if __name__ == "__main__":
    print(json.dumps({name: os.environ[name] for name in FIELDS if name in os.environ},
                     ensure_ascii=False, sort_keys=True))
