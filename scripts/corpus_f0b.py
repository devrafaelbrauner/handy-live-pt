#!/usr/bin/env python3
"""Corpus pessoal F0b (ULTRAPLAN §3): roteiros de leitura + texto-alvo.

    python3 scripts/corpus_f0b.py build    # fonte/*.txt -> manifest.jsonl, blocos/*.md, leitura.html
    python3 scripts/corpus_f0b.py status   # quais falas já foram gravadas no Handy

A fonte de verdade são os arquivos docs/corpus-f0b/fonte/bNN.txt. Formato:

    # b01 | Título do bloco
    mic: embutido | headset
    ambiente: silencioso | ruido
    nota: instrução mostrada antes do bloco

    @ f01 | tag tag tag
    leia: texto falado ({comando} dito em voz alta, _muleta_ dita e removida, ⏸ pausa de ~2 s)
    alvo:
    texto que deve sair colado (várias linhas permitidas)

    @ s01 | silencio      (sonda: segurar o atalho 5 s sem falar; alvo vazio)
"""

from __future__ import annotations

import argparse
import difflib
import json
import re
import sqlite3
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "docs" / "corpus-f0b"
FONTE = CORPUS / "fonte"
PAGE_TEMPLATE = Path(__file__).resolve().parent / "corpus_f0b_leitura.html"
HANDY_DB = Path.home() / "Library/Application Support/com.rafael.handylive/history.db"

# Ritmo de ditado lido (palavras/s), pausa marcada e custo fixo por fala
# (ler o trecho, apertar, soltar, esperar colar). Calibrado para estimativa,
# não para medida: o status real vem das gravações.
WORDS_PER_SEC = 2.0
PAUSE_SEC = 2.0
OVERHEAD_SEC = 6.0
SILENCE_SEC = 5.0

KNOWN_TAGS = {
    "pergunta", "lista", "paragrafos", "numeros", "ingles", "homofonos",
    "comandos", "muletas", "siglas", "concordancia", "pausa", "armadilha", "silencio",
}
# Cotas do ULTRAPLAN §3, F0b.
QUOTAS = {"pergunta": 30, "lista": 15, "paragrafos": 15, "numeros": 20, "ingles": 15, "homofonos": 15}

COMMAND_RE = re.compile(r"\{([^}]+)\}")
FILLER_RE = re.compile(r"_([^_]+)_")


def spoken_text(leia: str) -> str:
    """O que efetivamente sai da boca: comandos e muletas incluídos, sem marcação."""
    text = COMMAND_RE.sub(r" \1 ", leia)
    text = FILLER_RE.sub(r"\1", text)
    text = text.replace("⏸", " ")
    return re.sub(r"\s+", " ", text).strip()


def estimate_sec(item: dict) -> float:
    if "silencio" in item["tags"]:
        return SILENCE_SEC + OVERHEAD_SEC
    words = len(spoken_text(item["leia"]).split())
    return words / WORDS_PER_SEC + item["leia"].count("⏸") * PAUSE_SEC + OVERHEAD_SEC


def parse_block(path: Path) -> dict:
    lines = path.read_text(encoding="utf-8").splitlines()
    header = re.match(r"#\s*(b\d\d)\s*\|\s*(.+)", lines[0])
    if not header:
        raise SystemExit(f"{path.name}: primeira linha deve ser '# bNN | título'")
    block = {"id": header.group(1), "titulo": header.group(2).strip(), "falas": []}
    current: dict | None = None
    section = None
    for n, raw in enumerate(lines[1:], start=2):
        line = raw.rstrip()
        if line.startswith("@ "):
            fid, _, tags = line[2:].partition("|")
            current = {
                "id": f"{block['id']}-{fid.strip()}",
                "bloco": block["id"],
                "tags": tags.split(),
                "leia": "",
                "alvo": "",
            }
            unknown = set(current["tags"]) - KNOWN_TAGS
            if unknown:
                raise SystemExit(f"{path.name}:{n}: tags desconhecidas {sorted(unknown)}")
            block["falas"].append(current)
            section = None
        elif current is None:
            key, _, value = line.partition(":")
            if key in ("mic", "ambiente", "nota"):
                block[key] = value.strip()
        elif line.startswith("leia:"):
            section = "leia"
            current["leia"] = line[5:].strip()
        elif line == "alvo:":
            section = "alvo"
        elif section == "leia" and line:
            current["leia"] += " " + line.strip()
        elif section == "alvo":
            current["alvo"] += line + "\n"
    for fala in block["falas"]:
        fala["alvo"] = fala["alvo"].strip("\n")
        fala["mic"] = block.get("mic", "embutido")
        fala["ambiente"] = block.get("ambiente", "silencioso")
        silent = "silencio" in fala["tags"]
        if silent != (not fala["leia"]) or (not silent and not fala["alvo"]):
            raise SystemExit(f"{fala['id']}: fala precisa de leia+alvo; sonda de silêncio, de nenhum")
        if fala["leia"].count("{") != fala["leia"].count("}") or fala["leia"].count("_") % 2:
            raise SystemExit(f"{fala['id']}: marcação {{}} ou _ _ desbalanceada")
    return block


def load() -> list[dict]:
    blocks = [parse_block(p) for p in sorted(FONTE.glob("b*.txt"))]
    ids = [f["id"] for b in blocks for f in b["falas"]]
    dupes = {i for i in ids if ids.count(i) > 1}
    if dupes:
        raise SystemExit(f"ids duplicados: {sorted(dupes)}")
    return blocks


def fmt_min(sec: float) -> str:
    return f"{int(sec // 60)}min{int(sec % 60):02d}s"


def render_block_md(block: dict) -> str:
    total = sum(estimate_sec(f) for f in block["falas"])
    out = [
        f"# Bloco {block['id'][1:]} — {block['titulo']}",
        "",
        f"**Microfone:** {block.get('mic')} · **Ambiente:** {block.get('ambiente')} · **Duração estimada:** {fmt_min(total)}",
        "",
        "> " + FILLER_RE.sub(r"*\1*", COMMAND_RE.sub(r"**[\1]**", block.get("nota", ""))),
        "",
        "Legenda: **[comando]** é dito em voz alta · *muleta* é dita de propósito · ⏸ pausa de ~2 s.",
        "",
    ]
    for f in block["falas"]:
        fid = f["id"].split("-")[1]
        if "silencio" in f["tags"]:
            out += [f"### {fid} · silêncio", "", "Segure o atalho por **5 segundos sem falar** e solte.", ""]
            continue
        leia = COMMAND_RE.sub(r"**[\1]**", f["leia"])
        leia = FILLER_RE.sub(r"*\1*", leia)
        out += [f"### {fid} · {' '.join(f['tags'])}", "", leia, ""]
        out += ["<details><summary>texto-alvo</summary>", "", "```text", f["alvo"], "```", "", "</details>", ""]
    return "\n".join(out)


def cmd_build(_args) -> None:
    blocks = load()
    falas = [f for b in blocks for f in b["falas"]]
    with (CORPUS / "manifest.jsonl").open("w", encoding="utf-8") as fh:
        for f in falas:
            row = {**f, "falado": spoken_text(f["leia"]), "estimativa_s": round(estimate_sec(f), 1)}
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")
    for b in blocks:
        (CORPUS / "blocos" / f"bloco-{b['id'][1:]}.md").write_text(render_block_md(b), encoding="utf-8")

    page_data = [
        {k: b.get(k, "") for k in ("id", "titulo", "mic", "ambiente", "nota")}
        | {"falas": [{k: f[k] for k in ("id", "tags", "leia", "alvo")} | {"s": round(estimate_sec(f))} for f in b["falas"]]}
        for b in blocks
    ]
    template = PAGE_TEMPLATE.read_text(encoding="utf-8")
    (CORPUS / "leitura.html").write_text(
        template.replace("__CORPUS_JSON__", json.dumps(page_data, ensure_ascii=False)), encoding="utf-8"
    )

    speech = [f for f in falas if "silencio" not in f["tags"]]
    total = 0.0
    print(f"{'bloco':<6} {'falas':>5} {'silên.':>6} {'palavras':>8} {'estimativa':>11}  título")
    for b in blocks:
        sec = sum(estimate_sec(f) for f in b["falas"])
        total += sec
        sp = [f for f in b["falas"] if "silencio" not in f["tags"]]
        words = sum(len(spoken_text(f["leia"]).split()) for f in sp)
        print(f"{b['id']:<6} {len(sp):>5} {len(b['falas']) - len(sp):>6} {words:>8} {fmt_min(sec):>11}  {b['titulo']}")
    print(f"total: {len(speech)} falas + {len(falas) - len(speech)} sondas de silêncio, {fmt_min(total)} estimados")
    ok = True
    for tag, minimum in QUOTAS.items():
        n = sum(tag in f["tags"] for f in speech)
        mark = "ok" if n >= minimum else "FALTA"
        ok &= n >= minimum
        print(f"  cota {tag:<11} {n:>3} / {minimum:<3} {mark}")
    if not ok:
        sys.exit(1)


def normalize(text: str) -> str:
    text = unicodedata.normalize("NFKD", text.lower())
    text = "".join(c for c in text if not unicodedata.combining(c))
    return " ".join(re.sub(r"[^a-z0-9 ]+", " ", text).split())


def cmd_status(args) -> None:
    db = Path(args.db).expanduser()
    if not db.exists():
        raise SystemExit(f"histórico do Handy não encontrado em {db}")
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = conn.execute(
        "SELECT timestamp, file_name, transcription_text FROM transcription_history ORDER BY timestamp"
    ).fetchall()
    falas = [f for b in load() for f in b["falas"] if "silencio" not in f["tags"]]
    targets = [(f, normalize(spoken_text(f["leia"])), normalize(f["alvo"])) for f in falas]
    rec_dir = db.parent / "recordings"
    takes: dict[str, list] = {}
    short = 0
    for ts, file_name, text in rows:
        norm = normalize(text or "")
        if len((text or "").split()) < 4:
            short += 1
            continue
        best, score = None, 0.0
        for f, spoken, alvo in targets:
            s = max(difflib.SequenceMatcher(None, norm, spoken).ratio(), difflib.SequenceMatcher(None, norm, alvo).ratio())
            if s > score:
                best, score = f, s
        if best and score >= args.min_ratio:
            takes.setdefault(best["id"], []).append((ts, file_name, (rec_dir / file_name).exists()))
    done = [f for f in falas if f["id"] in takes]
    print(f"{len(done)}/{len(falas)} falas com gravação; {short} gravações curtas (prováveis sondas de silêncio)")
    by_block: dict[str, list] = {}
    for f in falas:
        by_block.setdefault(f["bloco"], []).append(f)
    for block_id, items in by_block.items():
        got = [f for f in items if f["id"] in takes]
        missing = [f["id"].split("-")[1] for f in items if f["id"] not in takes]
        print(f"  {block_id}: {len(got)}/{len(items)}" + (f"  faltam: {' '.join(missing)}" if missing else "  completo"))
    lost = [fid for fid, t in takes.items() if not any(exists for *_, exists in t)]
    if lost:
        print(f"ATENÇÃO: {len(lost)} falas estão no histórico mas o .wav foi apagado (retenção do Handy): {' '.join(lost)}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("build").set_defaults(func=cmd_build)
    status = sub.add_parser("status")
    status.add_argument("--db", default=str(HANDY_DB))
    status.add_argument("--min-ratio", type=float, default=0.55)
    status.set_defaults(func=cmd_status)
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
