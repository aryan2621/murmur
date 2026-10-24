#!/usr/bin/env python3
"""OpenVoice V2 sidecar (optional install). Loads the models once, then reads one JSON request per
line on stdin ({"ref", "text", "out"}) and answers each with one JSON line ({"ok": true} or
{"error": "..."}). Each answer is a mono WAV in the user's cloned voice at "out".

Needs: pip install openvoice-v2 melotts, and the V2 checkpoints unzipped to
~/Library/Application Support/Murmur/openvoice/checkpoints_v2 (or $MURMUR_OPENVOICE_CKPT).
Exit codes before serving: 2 packages missing, 3 checkpoints missing.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

CKPT = Path(os.environ.get("MURMUR_OPENVOICE_CKPT") or Path.home() / "Library/Application Support/Murmur/openvoice/checkpoints_v2")


def main() -> int:
    # Libraries print progress to stdout; keep it for replies only.
    replies, sys.stdout = sys.stdout, sys.stderr
    try:
        import torch  # type: ignore
        from melo.api import TTS  # type: ignore
        from openvoice import se_extractor  # type: ignore
        from openvoice.api import ToneColorConverter  # type: ignore
    except ImportError as e:
        print(f"Missing package ({e}). Run: pip install openvoice-v2 melotts", file=sys.stderr)
        return 2
    conv_dir = CKPT / "converter"
    source_se_path = CKPT / "base_speakers/ses/en-newest.pth"
    if not (conv_dir / "checkpoint.pth").is_file() or not source_se_path.is_file():
        print(f"OpenVoice checkpoints not found in {CKPT}", file=sys.stderr)
        return 3

    device = "mps" if torch.backends.mps.is_available() else "cpu"
    converter = ToneColorConverter(str(conv_dir / "config.json"), device=device)
    converter.load_ckpt(str(conv_dir / "checkpoint.pth"))
    source_se = torch.load(source_se_path, map_location=device)
    model = TTS(language="EN_NEWEST", device=device)
    speaker_id = next(iter(model.hps.data.spk2id.values()))

    def target_se(ref: Path):
        # Extracting the speaker embedding is the slow part; reuse it until the sample changes.
        cache = ref.with_suffix(".se.pth")
        if cache.is_file() and cache.stat().st_mtime >= ref.stat().st_mtime:
            return torch.load(cache, map_location=device)
        se, _ = se_extractor.get_se(str(ref), converter, vad=True)
        torch.save(se, cache)
        return se

    for line in sys.stdin:
        try:
            req = json.loads(line)
            ref = Path(req["ref"])
            if not ref.is_file():
                raise FileNotFoundError(f"reference missing: {ref}")
            with tempfile.TemporaryDirectory() as tmp:
                base = Path(tmp) / "base.wav"
                model.tts_to_file(req["text"], speaker_id, str(base), speed=1.0)
                converter.convert(audio_src_path=str(base), src_se=source_se, tgt_se=target_se(ref), output_path=req["out"], message="@MyShell")
            reply = {"ok": True}
        except Exception as e:  # one bad sentence must not end the session
            reply = {"error": str(e)}
        print(json.dumps(reply), file=replies, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
