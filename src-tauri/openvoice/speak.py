#!/usr/bin/env python3
"""Voice cloning sidecar (optional install). Loads one engine once, then reads one JSON request
per line on stdin ({"ref", "text", "out"}) and answers each with one JSON line ({"ok": true} or
{"error": "..."}). Each answer is a WAV at "out" in the voice of the reference sample.

Engines (--engine). Murmur installs chatterbox and f5 itself (Settings → Voice → Install), into
its own Python under voice-clone/ in its data folder; openvoice is for manual setups:
  chatterbox  chatterbox-tts             (default; 23 languages incl. Hindi, MIT)
  f5          f5-tts                     (high quality English/Chinese, slower start)
  openvoice   openvoice-v2 melotts       (+ checkpoints_v2 in $MURMUR_OPENVOICE_CKPT)
Exit codes before serving: 2 package missing, 3 model files missing, 4 unknown engine.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path

INSTALL = {"chatterbox": "pip install chatterbox-tts", "f5": "pip install f5-tts", "openvoice": "pip install openvoice-v2 melotts"}


class Missing(Exception):
    """Exit code with a message, before serving."""

    def __init__(self, code: int, message: str):
        super().__init__(message)
        self.code = code


def device(torch) -> str:
    return "mps" if torch.backends.mps.is_available() else "cpu"


def chatterbox():
    try:
        import torch  # type: ignore
        import torchaudio  # type: ignore
        from chatterbox.mtl_tts import ChatterboxMultilingualTTS  # type: ignore
    except ImportError as e:
        raise Missing(2, f"Missing package ({e}). Run: {INSTALL['chatterbox']}")
    dev = device(torch)
    # Its checkpoints were saved on CUDA; load them onto this Mac's device.
    load = torch.load
    torch.load = lambda *a, **k: load(*a, **{**k, "map_location": torch.device(dev)})
    model = ChatterboxMultilingualTTS.from_pretrained(device=dev)

    def synth(ref: Path, text: str, out: str):
        hindi = any("ऀ" <= c <= "ॿ" for c in text)
        wav = model.generate(text, language_id="hi" if hindi else "en", audio_prompt_path=str(ref))
        torchaudio.save(out, wav.cpu(), model.sr)

    return synth


def f5():
    try:
        import torch  # type: ignore
        from f5_tts.api import F5TTS  # type: ignore
    except ImportError as e:
        raise Missing(2, f"Missing package ({e}). Run: {INSTALL['f5']}")
    model = F5TTS(device=device(torch))

    def synth(ref: Path, text: str, out: str):
        # F5 needs the words of the sample; transcribing it is slow, so do it once per sample.
        words = ref.with_suffix(".txt")
        if not words.is_file() or words.stat().st_mtime < ref.stat().st_mtime:
            words.write_text(model.transcribe(str(ref)))
        model.infer(ref_file=str(ref), ref_text=words.read_text(), gen_text=text, file_wave=out)

    return synth


def openvoice():
    try:
        import torch  # type: ignore
        from melo.api import TTS  # type: ignore
        from openvoice import se_extractor  # type: ignore
        from openvoice.api import ToneColorConverter  # type: ignore
    except ImportError as e:
        raise Missing(2, f"Missing package ({e}). Run: {INSTALL['openvoice']}")
    ckpt = Path(os.environ.get("MURMUR_OPENVOICE_CKPT") or Path.home() / "Library/Application Support/Murmur/openvoice/checkpoints_v2")
    conv_dir = ckpt / "converter"
    source_se_path = ckpt / "base_speakers/ses/en-newest.pth"
    if not (conv_dir / "checkpoint.pth").is_file() or not source_se_path.is_file():
        raise Missing(3, f"OpenVoice checkpoints not found in {ckpt}")
    dev = device(torch)
    converter = ToneColorConverter(str(conv_dir / "config.json"), device=dev)
    converter.load_ckpt(str(conv_dir / "checkpoint.pth"))
    source_se = torch.load(source_se_path, map_location=dev)
    model = TTS(language="EN_NEWEST", device=dev)
    speaker_id = next(iter(model.hps.data.spk2id.values()))

    def target_se(ref: Path):
        # Extracting the speaker embedding is the slow part; reuse it until the sample changes.
        cache = ref.with_suffix(".se.pth")
        if cache.is_file() and cache.stat().st_mtime >= ref.stat().st_mtime:
            return torch.load(cache, map_location=dev)
        se, _ = se_extractor.get_se(str(ref), converter, vad=True)
        torch.save(se, cache)
        return se

    def synth(ref: Path, text: str, out: str):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp) / "base.wav"
            model.tts_to_file(text, speaker_id, str(base), speed=1.0)
            converter.convert(audio_src_path=str(base), src_se=source_se, tgt_se=target_se(ref), output_path=out, message="@MyShell")

    return synth


ENGINES = {"chatterbox": chatterbox, "f5": f5, "openvoice": openvoice}


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--engine", default="chatterbox")
    p.add_argument("--prefetch", action="store_true", help="Load the engine (downloading its weights) and exit")
    args = p.parse_args()
    # Libraries print progress to stdout; keep it for replies only.
    replies, sys.stdout = sys.stdout, sys.stderr
    if args.engine not in ENGINES:
        print(f"Unknown engine {args.engine}; choose one of {', '.join(ENGINES)}", file=sys.stderr)
        return 4
    try:
        synth = ENGINES[args.engine]()
    except Missing as e:
        print(e, file=sys.stderr)
        return e.code
    if args.prefetch:
        return 0

    for line in sys.stdin:
        try:
            req = json.loads(line)
            ref = Path(req["ref"])
            if not ref.is_file():
                raise FileNotFoundError(f"reference missing: {ref}")
            synth(ref, req["text"], req["out"])
            reply = {"ok": True}
        except Exception as e:  # one bad sentence must not end the session
            reply = {"error": str(e)}
        print(json.dumps(reply), file=replies, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
