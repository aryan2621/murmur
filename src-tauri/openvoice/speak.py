#!/usr/bin/env python3
"""OpenVoice sidecar (optional pip install). Writes 24 kHz mono WAV for Murmur's player."""

from __future__ import annotations

import argparse
import sys
import wave
from pathlib import Path


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--ref", required=True, help="Reference speaker WAV (16 kHz mono)")
    p.add_argument("--text", required=True)
    p.add_argument("--out", required=True)
    args = p.parse_args()
    ref = Path(args.ref)
    out = Path(args.out)
    if not ref.is_file():
        print(f"reference missing: {ref}", file=sys.stderr)
        return 1
    try:
        from openvoice.api import ToneColorConverter  # type: ignore
    except ImportError:
        print("OpenVoice is not installed. Run: pip install openvoice-v2", file=sys.stderr)
        return 2
    # Placeholder: real OpenVoice wiring depends on the user's installed checkpoint paths.
    # v1 sidecar proves subprocess + reference storage; extend with bundled weights later.
    print("OpenVoice package found but bundled inference is not configured in this build.", file=sys.stderr)
    return 3


if __name__ == "__main__":
    raise SystemExit(main())
