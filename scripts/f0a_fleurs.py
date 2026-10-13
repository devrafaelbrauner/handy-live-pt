#!/usr/bin/env python3
"""FLEURS pt_br para a linha de base da F0a (docs/ULTRAPLAN.md §3): baixa N falas
do split de teste, converte para o WAV que o harness aceita (16 kHz mono
16-bit) e escreve o manifest.

    python3 scripts/f0a_fleurs.py [--out ~/handy-f0a/fleurs-pt_br] [--count 200]
    handy --manifest ~/handy-f0a/fleurs-pt_br/manifest.jsonl --model ID --language pt --vad silero --out r.jsonl
    python3 scripts/f0a_metrics.py r.jsonl --manifest ~/handy-f0a/fleurs-pt_br/manifest.jsonl

Seleção: uma gravação por frase, as primeiras em ordem de nome de arquivo (os
nomes são hashes, então a ordem é arbitrária, mas estável entre execuções).
Os áudios vêm do `test.tar.gz` do dataset google/fleurs no Hugging Face
(~620 MB, lido em streaming até achar as falas escolhidas; nada dele é
guardado além delas). Uma nova execução só baixa o que ainda falta.

Manifest (uma linha por fala): `id` (fleurs-<arquivo>), `audio` (wav/<arquivo>,
relativo ao manifest), `alvo` (raw_transcription, com caixa e pontuação),
`normalizado` (transcription), `frase`, `genero`, `duracao_s`.
Licença do FLEURS: CC BY 4.0.
"""

from __future__ import annotations

import argparse
import array
import csv
import io
import json
import struct
import sys
import tarfile
import urllib.request
import wave
from pathlib import Path

BASE_URL = "https://huggingface.co/datasets/google/fleurs/resolve/main/data/pt_br"
SAMPLE_RATE = 16_000
TSV_FIELDS = ("frase", "file_name", "raw", "normalized", "chars", "num_samples", "gender")


def _open(url: str):
    return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "handy-live-pt-f0a"}))


def select(rows: list[dict], count: int) -> list[dict]:
    """One recording per sentence, the first ones in file-name order."""
    chosen, sentences = [], set()
    for row in sorted(rows, key=lambda r: r["file_name"]):
        if row["frase"] not in sentences:
            sentences.add(row["frase"])
            chosen.append(row)
            if len(chosen) == count:
                break
    return chosen


def to_pcm16(data: bytes) -> bytes:
    """WAV bytes (16 kHz mono, IEEE float32 as FLEURS ships, or PCM 16-bit) →
    16 kHz mono 16-bit PCM WAV bytes."""
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE file")
    fmt = samples = None
    offset = 12
    while offset + 8 <= len(data):
        chunk_id, size = data[offset : offset + 4], struct.unpack_from("<I", data, offset + 4)[0]
        body = data[offset + 8 : offset + 8 + size]
        if chunk_id == b"fmt ":
            fmt = struct.unpack_from("<HHIIHH", body)
        elif chunk_id == b"data":
            samples = body
        offset += 8 + size + (size & 1)
    if fmt is None or samples is None:
        raise ValueError("missing fmt or data chunk")
    tag, channels, rate, _, _, bits = fmt
    if channels != 1 or rate != SAMPLE_RATE:
        raise ValueError(f"expected 16 kHz mono, got {rate} Hz / {channels} ch")
    if (tag, bits) == (1, 16):
        pcm = array.array("h", samples)
    elif (tag, bits) == (3, 32):
        floats = array.array("f", samples)
        if sys.byteorder == "big":
            floats.byteswap()
        pcm = array.array("h", (int(round(max(-1.0, min(1.0, x)) * 32767)) for x in floats))
    else:
        raise ValueError(f"unsupported WAV encoding (format {tag}, {bits}-bit)")
    if sys.byteorder == "big":
        pcm.byteswap()
    out = io.BytesIO()
    with wave.open(out, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(SAMPLE_RATE)
        wav.writeframes(pcm.tobytes())
    return out.getvalue()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", type=Path, default=Path.home() / "handy-f0a" / "fleurs-pt_br")
    parser.add_argument("--count", type=int, default=200, help="falas (padrão: 200)")
    parser.add_argument("--split", default="test", choices=("test", "dev", "train"))
    args = parser.parse_args(argv)

    out = args.out.expanduser()
    wav_dir = out / "wav"
    wav_dir.mkdir(parents=True, exist_ok=True)

    with _open(f"{BASE_URL}/{args.split}.tsv") as resp:
        reader = csv.reader(io.TextIOWrapper(resp, encoding="utf-8"), delimiter="\t", quoting=csv.QUOTE_NONE)
        rows = [dict(zip(TSV_FIELDS, fields)) for fields in reader if len(fields) >= len(TSV_FIELDS)]
    chosen = select(rows, args.count)
    if len(chosen) < args.count:
        print(f"aviso: o split {args.split} só tem {len(chosen)} frases distintas", file=sys.stderr)

    pending = {row["file_name"] for row in chosen if not (wav_dir / row["file_name"]).exists()}
    if pending:
        print(f"baixando {len(pending)} de {len(chosen)} falas de {BASE_URL}/audio/{args.split}.tar.gz …")
        with _open(f"{BASE_URL}/audio/{args.split}.tar.gz") as resp, tarfile.open(fileobj=resp, mode="r|gz") as tar:
            for member in tar:
                name = Path(member.name).name
                if not member.isfile() or name not in pending:
                    continue
                (wav_dir / name).write_bytes(to_pcm16(tar.extractfile(member).read()))
                pending.discard(name)
                done = len(chosen) - len(pending)
                if done % 25 == 0:
                    print(f"  {done}/{len(chosen)}")
                if not pending:
                    break
        if pending:
            print(f"error: {len(pending)} arquivos não estão no tar: {sorted(pending)[:5]}", file=sys.stderr)
            return 1

    with (out / "manifest.jsonl").open("w", encoding="utf-8") as fh:
        for row in chosen:
            fh.write(
                json.dumps(
                    {
                        "id": f"fleurs-{Path(row['file_name']).stem}",
                        "audio": f"wav/{row['file_name']}",
                        "alvo": row["raw"],
                        "normalizado": row["normalized"],
                        "frase": row["frase"],
                        "genero": row["gender"],
                        "duracao_s": round(int(row["num_samples"]) / SAMPLE_RATE, 2),
                    },
                    ensure_ascii=False,
                )
                + "\n"
            )
    total = sum(int(row["num_samples"]) for row in chosen) / SAMPLE_RATE
    print(f"{len(chosen)} falas ({total / 60:.1f} min) em {out / 'manifest.jsonl'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
