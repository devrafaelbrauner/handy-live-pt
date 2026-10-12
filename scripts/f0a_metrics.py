#!/usr/bin/env python3
"""Métricas da F0a (docs/ULTRAPLAN.md §3 e §4) sobre a saída do harness `handy --manifest`.

    handy --manifest m.jsonl --model ID --language pt --vad silero > resultados.jsonl
    python3 scripts/f0a_metrics.py resultados.jsonl --manifest m.jsonl [--ref alvo] [--hyp final] [--json relatorio.json]
    python3 scripts/f0a_metrics.py candidato.jsonl --manifest m.jsonl --baseline base.jsonl [--samples 2000] [--seed 0]

Cada linha `"type": "utterance"` do harness é juntada pelo `id` com a entrada do
manifest (o mesmo arquivo passado ao harness, ou outro com os mesmos ids; sem
`id`, vale o `audio`, como no harness). Compara o texto do harness (`final`, ou
`raw` com --hyp raw) com o campo de referência do manifest (`alvo` por padrão).

Todas as taxas somam as falas (micro) e saem também por faixa de duração do
áudio (≤ 10 s, 10–30 s, > 30 s):

- WER/CER normalizado: NFC + casefold, sem pontuação nem símbolos; números ficam
  como escritos (`18h30` ≠ "dezoito e trinta"). O CER conta os caracteres da
  forma normalizada, com um espaço entre palavras.
- WER formatado: diferencia caixa e conta `, . ? ! : ;` como palavras.
- Pontuação: P/R/F1 de `, . ? ! :`. Alinha as palavras normalizadas e compara o
  sinal que segue cada palavra da hipótese com o da palavra de referência
  alinhada (`?!` conta como `?`, `...` e `…` como `.`).
- ITN exact-match: trechos numéricos da referência (`18h30`, `18:30`,
  `R$ 15.000,00`, `10%`, `1º`, `3,5`) encontrados idênticos na hipótese;
  espúrios são trechos da hipótese que a referência não tem.
- Preservação: edições entre as palavras normalizadas de `raw` e `final`, ou
  seja, o que os estágios de texto mudaram.
- Sondas de silêncio: referência vazia. Ficam fora das taxas acima; conta as
  palavras que a hipótese inseriu.

Com --baseline, compara o candidato com a linha de base (§4): para cada taxa e
faixa, Δ = candidato − base nas falas avaliadas nas duas execuções, com IC 95%
por bootstrap pareado (reamostra as falas com reposição; percentis 2,5 e 97,5).
O F1 de pontuação comparado é o micro de todos os sinais, 2·TP / (2·TP + FP + FN).

Saída: relatório em texto; com --json, também o relatório completo com as
contagens por fala. Código de saída 2 para entrada inválida.
"""

from __future__ import annotations

import argparse
import json
import math
import random
import re
import sys
import unicodedata
from collections import Counter
from pathlib import Path

WORD = re.compile(r"\w+(?:[-'’.,:/]\w+)*")
FORMATTED_TOKEN = re.compile(r"\w+(?:[-'’.,:/]\w+)*|[,.?!:;]")
MARKS = (",", ".", "?", "!", ":")
# Ordem de prioridade quando há mais de um sinal depois da palavra.
MARK_PRIORITY = (("?", "?"), ("!", "!"), (":", ":"), (".", "."), ("…", "."), (",", ","))
ITN_SPAN = re.compile(
    r"R\$\s?[0-9]+(?:[.,][0-9]+)*"
    r"|(?<!\w)[0-9]+(?:[.,:/][0-9]+)*(?:h(?:[0-9]{2})?|º|ª|°|\s?%)?(?!\w)"
)
# (name, upper bound in seconds of audio), ascending.
BANDS = (("≤ 10 s", 10.0), ("10–30 s", 30.0), ("> 30 s", math.inf))


class InputError(Exception):
    """Invalid results or manifest file."""


# ---------------------------------------------------------------------------
# Texto


def _nfc(text: str) -> str:
    return unicodedata.normalize("NFC", text)


def normalized_words(text: str) -> list[str]:
    return WORD.findall(_nfc(text).casefold())


def formatted_tokens(text: str) -> list[str]:
    return FORMATTED_TOKEN.findall(_nfc(text))


def words_with_marks(text: str) -> list[tuple[str, str | None]]:
    """Each normalized word with the tracked mark that follows it, if any."""
    text = _nfc(text)
    matches = list(WORD.finditer(text))
    out = []
    for index, match in enumerate(matches):
        gap_end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        gap = text[match.end() : gap_end]
        mark = next((name for char, name in MARK_PRIORITY if char in gap), None)
        out.append((match.group().casefold(), mark))
    return out


def itn_spans(text: str) -> list[str]:
    return ITN_SPAN.findall(_nfc(text))


# ---------------------------------------------------------------------------
# Alinhamento


def align(ref: list, hyp: list) -> tuple[int, list[tuple[int | None, int | None]]]:
    """Levenshtein distance and one minimal alignment as (ref index, hyp index)
    pairs; `None` on one side is a deletion (hyp) or insertion (ref)."""
    rows, cols = len(ref) + 1, len(hyp) + 1
    dist = [[0] * cols for _ in range(rows)]
    for i in range(rows):
        dist[i][0] = i
    for j in range(cols):
        dist[0][j] = j
    for i in range(1, rows):
        for j in range(1, cols):
            cost = 0 if ref[i - 1] == hyp[j - 1] else 1
            dist[i][j] = min(dist[i - 1][j - 1] + cost, dist[i - 1][j] + 1, dist[i][j - 1] + 1)

    pairs: list[tuple[int | None, int | None]] = []
    i, j = len(ref), len(hyp)
    while i > 0 or j > 0:
        if i > 0 and j > 0 and dist[i][j] == dist[i - 1][j - 1] + (ref[i - 1] != hyp[j - 1]):
            pairs.append((i - 1, j - 1))
            i, j = i - 1, j - 1
        elif i > 0 and dist[i][j] == dist[i - 1][j] + 1:
            pairs.append((i - 1, None))
            i -= 1
        else:
            pairs.append((None, j - 1))
            j -= 1
    pairs.reverse()
    return dist[len(ref)][len(hyp)], pairs


def edit_distance(ref: list, hyp: list) -> int:
    previous = list(range(len(hyp) + 1))
    for i, ref_item in enumerate(ref, 1):
        current = [i]
        for j, hyp_item in enumerate(hyp, 1):
            current.append(
                min(previous[j - 1] + (ref_item != hyp_item), previous[j] + 1, current[j - 1] + 1)
            )
        previous = current
    return previous[-1]


def punctuation_counts(ref_text: str, hyp_text: str) -> dict[str, Counter]:
    """Per-mark true positives, false positives and false negatives."""
    ref = words_with_marks(ref_text)
    hyp = words_with_marks(hyp_text)
    _, pairs = align([word for word, _ in ref], [word for word, _ in hyp])
    counts = {mark: Counter() for mark in MARKS}
    for ref_index, hyp_index in pairs:
        ref_mark = ref[ref_index][1] if ref_index is not None else None
        hyp_mark = hyp[hyp_index][1] if hyp_index is not None else None
        if ref_mark is not None and ref_mark == hyp_mark:
            counts[ref_mark]["tp"] += 1
            continue
        if hyp_mark is not None:
            counts[hyp_mark]["fp"] += 1
        if ref_mark is not None:
            counts[ref_mark]["fn"] += 1
    return counts


# ---------------------------------------------------------------------------
# Entrada


def _read_jsonl(path: Path) -> list[tuple[int, dict]]:
    try:
        text = path.read_text(encoding="utf-8-sig")
    except OSError as e:
        raise InputError(f"cannot read {path}: {e}") from e
    rows = []
    for line_no, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as e:
            raise InputError(f"{path}:{line_no}: invalid JSON: {e}") from e
        if not isinstance(row, dict):
            raise InputError(f"{path}:{line_no}: expected a JSON object")
        rows.append((line_no, row))
    return rows


def load_manifest(path: Path, ref_field: str) -> dict[str, str]:
    refs: dict[str, str] = {}
    for line_no, row in _read_jsonl(path):
        utterance_id = row.get("id", row.get("audio"))
        if not isinstance(utterance_id, str) or not utterance_id:
            raise InputError(f'{path}:{line_no}: needs a string "id" (or "audio")')
        ref = row.get(ref_field)
        if not isinstance(ref, str):
            raise InputError(f'{path}:{line_no}: "{utterance_id}" has no string field "{ref_field}"')
        if utterance_id in refs:
            raise InputError(f'{path}:{line_no}: duplicate id "{utterance_id}"')
        refs[utterance_id] = ref
    return refs


def load_results(path: Path) -> tuple[list[dict], list[dict], dict | None]:
    """Scored utterances, failed utterances and the harness summary line."""
    utterances: list[dict] = []
    failed: list[dict] = []
    summary = None
    seen = set()
    for line_no, row in _read_jsonl(path):
        kind = row.get("type")
        if kind == "summary":
            summary = row
            continue
        if kind != "utterance":
            raise InputError(f'{path}:{line_no}: unknown "type" {kind!r}')
        utterance_id = row.get("id")
        if not isinstance(utterance_id, str) or utterance_id in seen:
            raise InputError(f'{path}:{line_no}: missing or duplicate "id"')
        seen.add(utterance_id)
        if "error" in row:
            failed.append(row)
            continue
        for field in ("raw", "final", "audio_secs"):
            if field not in row:
                raise InputError(f'{path}:{line_no}: "{utterance_id}" has no "{field}"')
        utterances.append(row)
    return utterances, failed, summary


# ---------------------------------------------------------------------------
# Métricas


def band_of(audio_secs: float) -> str:
    return next(name for name, upper in BANDS if audio_secs <= upper)


def score_utterance(row: dict, ref: str, hyp_field: str) -> dict:
    hyp = row[hyp_field]
    ref_words = normalized_words(ref)
    hyp_words = normalized_words(hyp)
    raw_words = normalized_words(row["raw"])
    final_words = normalized_words(row["final"])
    scored = {
        "id": row["id"],
        "audio_secs": row["audio_secs"],
        "band": band_of(row["audio_secs"]),
        "silence_probe": not ref_words,
        "raw_words": len(raw_words),
        "preservation_edits": edit_distance(raw_words, final_words),
    }
    if not ref_words:
        scored["inserted_words"] = len(hyp_words)
        return scored

    ref_chars = list(" ".join(ref_words))
    ref_tokens = formatted_tokens(ref)
    ref_spans = itn_spans(ref)
    hyp_spans = itn_spans(hyp)
    matched = sum((Counter(ref_spans) & Counter(hyp_spans)).values())
    scored.update(
        {
            "ref_words": len(ref_words),
            "wer_edits": edit_distance(ref_words, hyp_words),
            "ref_chars": len(ref_chars),
            "cer_edits": edit_distance(ref_chars, list(" ".join(hyp_words))),
            "ref_tokens": len(ref_tokens),
            "formatted_edits": edit_distance(ref_tokens, formatted_tokens(hyp)),
            "itn_ref": len(ref_spans),
            "itn_matched": matched,
            "itn_spurious": len(hyp_spans) - matched,
            "punctuation": {
                mark: dict(counts) for mark, counts in punctuation_counts(ref, hyp).items()
            },
        }
    )
    return scored


def _rate(numerator: int, denominator: int) -> float | None:
    return numerator / denominator if denominator else None


def _rates(rows: list[dict]) -> dict:
    speech = [row for row in rows if not row["silence_probe"]]

    def total(key: str) -> int:
        return sum(row[key] for row in speech)

    return {
        "utterances": len(speech),
        "wer": _rate(total("wer_edits"), total("ref_words")),
        "cer": _rate(total("cer_edits"), total("ref_chars")),
        "formatted_wer": _rate(total("formatted_edits"), total("ref_tokens")),
    }


def _prf(counts: Counter) -> dict:
    tp, fp, fn = counts["tp"], counts["fp"], counts["fn"]
    precision = _rate(tp, tp + fp)
    recall = _rate(tp, tp + fn)
    f1 = (
        2 * precision * recall / (precision + recall)
        if precision is not None and recall is not None and precision + recall > 0
        else None
    )
    return {"tp": tp, "fp": fp, "fn": fn, "precision": precision, "recall": recall, "f1": f1}


def build_report(results: Path, manifest: Path, ref_field: str, hyp_field: str) -> dict:
    refs = load_manifest(manifest, ref_field)
    utterances, failed, summary = load_results(results)
    unknown = [row["id"] for row in utterances + failed if row["id"] not in refs]
    if unknown:
        raise InputError(f"{results}: ids not in {manifest}: {', '.join(unknown[:10])}")

    scored = [score_utterance(row, refs[row["id"]], hyp_field) for row in utterances]
    reported = {row["id"] for row in utterances + failed}
    speech = [row for row in scored if not row["silence_probe"]]
    probes = [row for row in scored if row["silence_probe"]]

    punctuation_totals = {mark: Counter() for mark in MARKS}
    for row in speech:
        for mark, counts in row["punctuation"].items():
            punctuation_totals[mark].update(counts)
    punctuation = {mark: _prf(counts) for mark, counts in punctuation_totals.items()}
    punctuation["all"] = _prf(sum(punctuation_totals.values(), Counter()))

    itn_ref = sum(row["itn_ref"] for row in speech)
    itn_matched = sum(row["itn_matched"] for row in speech)
    raw_words = sum(row["raw_words"] for row in scored)
    preservation_edits = sum(row["preservation_edits"] for row in scored)

    return {
        "inputs": {
            "results": str(results),
            "manifest": str(manifest),
            "ref": ref_field,
            "hyp": hyp_field,
            "harness": summary,
        },
        "counts": {
            "scored": len(speech),
            "silence_probes": len(probes),
            "failed": [row["id"] for row in failed],
            "missing": [utterance_id for utterance_id in refs if utterance_id not in reported],
        },
        "rates": {
            "all": _rates(scored),
            **{name: _rates([row for row in scored if row["band"] == name]) for name, _ in BANDS},
        },
        "punctuation": punctuation,
        "itn": {
            "ref_spans": itn_ref,
            "matched": itn_matched,
            "exact_match": _rate(itn_matched, itn_ref),
            "spurious": sum(row["itn_spurious"] for row in speech),
        },
        "preservation": {
            "raw_words": raw_words,
            "edits": preservation_edits,
            "rate": _rate(preservation_edits, raw_words),
            "utterances_changed": sum(1 for row in scored if row["preservation_edits"]),
        },
        "silence": {
            "probes": len(probes),
            "inserted_words": sum(row["inserted_words"] for row in probes),
            "probes_with_words": sum(1 for row in probes if row["inserted_words"]),
        },
        "utterances": scored,
    }


# ---------------------------------------------------------------------------
# Comparação com a linha de base

# (key, label, higher is better)
COMPARED = (
    ("wer", "WER normalizado", False),
    ("cer", "CER normalizado", False),
    ("formatted_wer", "WER formatado", False),
    ("punctuation_f1", "F1 pontuação", True),
    ("itn_exact", "ITN exact-match", True),
)


def _counts_vector(row: dict) -> tuple[int, ...]:
    marks = row["punctuation"].values()
    return (
        row["wer_edits"],
        row["ref_words"],
        row["cer_edits"],
        row["ref_chars"],
        row["formatted_edits"],
        row["ref_tokens"],
        sum(counts.get("tp", 0) for counts in marks),
        sum(counts.get("fp", 0) for counts in marks),
        sum(counts.get("fn", 0) for counts in marks),
        row["itn_matched"],
        row["itn_ref"],
    )


def _metrics_of(vectors: list[tuple[int, ...]]) -> dict[str, float | None]:
    wer_e, words, cer_e, chars, fmt_e, tokens, tp, fp, fn, itn_m, itn_r = map(sum, zip(*vectors))
    return {
        "wer": _rate(wer_e, words),
        "cer": _rate(cer_e, chars),
        "formatted_wer": _rate(fmt_e, tokens),
        "punctuation_f1": _rate(2 * tp, 2 * tp + fp + fn),
        "itn_exact": _rate(itn_m, itn_r),
    }


def _percentile(sorted_values: list[float], q: float) -> float:
    position = (len(sorted_values) - 1) * q
    low = math.floor(position)
    high = math.ceil(position)
    return sorted_values[low] + (sorted_values[high] - sorted_values[low]) * (position - low)


def _compare_group(base: list[tuple], cand: list[tuple], samples: int, rng: random.Random) -> dict:
    result: dict = {"utterances": len(base)}
    if not base:
        return result
    base_point, cand_point = _metrics_of(base), _metrics_of(cand)
    deltas: dict[str, list[float]] = {key: [] for key, _, _ in COMPARED}
    indices = range(len(base))
    for _ in range(samples):
        picked = rng.choices(indices, k=len(base))
        base_sample = _metrics_of([base[i] for i in picked])
        cand_sample = _metrics_of([cand[i] for i in picked])
        for key, values in deltas.items():
            if base_sample[key] is not None and cand_sample[key] is not None:
                values.append(cand_sample[key] - base_sample[key])
    for key, _, higher_is_better in COMPARED:
        b, c = base_point[key], cand_point[key]
        values = sorted(deltas[key])
        ci = [_percentile(values, 0.025), _percentile(values, 0.975)] if values else None
        verdict = None
        if ci is not None and (ci[0] > 0 or ci[1] < 0):
            verdict = "melhor" if (ci[0] > 0) == higher_is_better else "pior"
        result[key] = {
            "baseline": b,
            "candidate": c,
            "delta": c - b if b is not None and c is not None else None,
            "ci95": ci,
            "verdict": verdict,
        }
    return result


def compare_reports(base: dict, cand: dict, samples: int, seed: int) -> dict:
    """Paired comparison of two reports over the same manifest (cand − base)."""
    base_rows = {row["id"]: row for row in base["utterances"]}
    cand_rows = {row["id"]: row for row in cand["utterances"]}
    paired = [i for i in base_rows if i in cand_rows]
    speech = [i for i in paired if not base_rows[i]["silence_probe"]]
    probes = [i for i in paired if base_rows[i]["silence_probe"]]

    rng = random.Random(seed)
    groups = {"all": speech, **{name: [i for i in speech if base_rows[i]["band"] == name] for name, _ in BANDS}}
    return {
        "baseline": base["inputs"]["results"],
        "samples": samples,
        "seed": seed,
        "paired": len(speech),
        "only_baseline": sorted(set(base_rows) - set(cand_rows)),
        "only_candidate": sorted(set(cand_rows) - set(base_rows)),
        "groups": {
            name: _compare_group(
                [_counts_vector(base_rows[i]) for i in ids],
                [_counts_vector(cand_rows[i]) for i in ids],
                samples,
                rng,
            )
            for name, ids in groups.items()
        },
        "silence": {
            "probes": len(probes),
            "baseline_inserted_words": sum(base_rows[i]["inserted_words"] for i in probes),
            "candidate_inserted_words": sum(cand_rows[i]["inserted_words"] for i in probes),
        },
    }


# ---------------------------------------------------------------------------
# Relatório


def _pct(value: float | None) -> str:
    return "—" if value is None else f"{100 * value:.1f}%"


def _dec(value: float | None) -> str:
    return "—" if value is None else f"{value:.3f}"


def render(report: dict) -> str:
    inputs, counts = report["inputs"], report["counts"]
    lines = [f"F0a — {inputs['results']} × {inputs['manifest']} (ref: {inputs['ref']}, hyp: {inputs['hyp']})"]
    harness = inputs["harness"]
    if harness:
        lines.append(
            f"harness: model {harness.get('model')}, vad {harness.get('vad')}, "
            f"language {harness.get('language')}, backend {harness.get('bound_backend')}"
        )
    lines.append(
        f"falas: {counts['scored']} avaliadas, {counts['silence_probes']} sondas de silêncio, "
        f"{len(counts['failed'])} com erro, {len(counts['missing'])} sem resultado"
    )
    for label, key in (("com erro", "failed"), ("sem resultado", "missing")):
        if counts[key]:
            lines.append(f"  {label}: {', '.join(counts[key][:20])}{' …' if len(counts[key]) > 20 else ''}")

    columns = ["all"] + [name for name, _ in BANDS]
    rates = report["rates"]
    lines += ["", f"{'':16}" + "".join(f"{('todas' if c == 'all' else c):>10}" for c in columns)]
    lines.append(f"{'falas':16}" + "".join(f"{rates[c]['utterances']:>10}" for c in columns))
    for label, key in (("WER normalizado", "wer"), ("CER normalizado", "cer"), ("WER formatado", "formatted_wer")):
        lines.append(f"{label:16}" + "".join(f"{_pct(rates[c][key]):>10}" for c in columns))

    lines += ["", f"{'pontuação':10}{'P':>8}{'R':>8}{'F1':>8}{'TP':>6}{'FP':>6}{'FN':>6}"]
    for mark, row in report["punctuation"].items():
        label = "todos" if mark == "all" else mark
        lines.append(
            f"{label:10}{_dec(row['precision']):>8}{_dec(row['recall']):>8}{_dec(row['f1']):>8}"
            f"{row['tp']:>6}{row['fp']:>6}{row['fn']:>6}"
        )

    itn, preservation, silence = report["itn"], report["preservation"], report["silence"]
    lines += [
        "",
        f"ITN exact-match: {itn['matched']}/{itn['ref_spans']} ({_pct(itn['exact_match'])}), "
        f"espúrios: {itn['spurious']}",
        f"preservação (raw → final): {preservation['edits']} edições em {preservation['raw_words']} "
        f"palavras ({_pct(preservation['rate'])}), {preservation['utterances_changed']} falas alteradas",
        f"sondas de silêncio: {silence['probes']}, palavras inseridas: {silence['inserted_words']} "
        f"(em {silence['probes_with_words']} sondas)",
    ]
    return "\n".join(lines)


def _pp(value: float | None) -> str:
    return "—" if value is None else f"{100 * value:+.1f} pp".replace("-", "−")


def render_comparison(comparison: dict) -> str:
    lines = [
        f"Δ vs linha de base {comparison['baseline']}: {comparison['paired']} falas pareadas "
        f"({len(comparison['only_baseline'])} só na base, {len(comparison['only_candidate'])} só no candidato); "
        f"bootstrap pareado, {comparison['samples']} amostras, seed {comparison['seed']}",
        "",
        f"{'faixa':9}{'métrica':17}{'base':>9}{'cand.':>9}{'Δ':>11}{'IC 95%':>25}",
    ]
    for group, result in comparison["groups"].items():
        name = "todas" if group == "all" else group
        if not result["utterances"]:
            lines.append(f"{name:9}(nenhuma fala)")
            continue
        for key, label, _ in COMPARED:
            row = result[key]
            ci = "—" if row["ci95"] is None else f"[{_pp(row['ci95'][0])}, {_pp(row['ci95'][1])}]"
            verdict = row["verdict"] or "≈"
            lines.append(
                f"{name:9}{label:17}{_pct(row['baseline']):>9}{_pct(row['candidate']):>9}"
                f"{_pp(row['delta']):>11}{ci:>25}  {verdict}"
            )
            name = ""
    silence = comparison["silence"]
    lines += [
        "",
        f"sondas de silêncio pareadas: {silence['probes']}; palavras inseridas: "
        f"base {silence['baseline_inserted_words']}, candidato {silence['candidate_inserted_words']}",
    ]
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("results", type=Path, help="saída JSONL do harness (handy --manifest)")
    parser.add_argument("--manifest", type=Path, required=True, help="manifest JSONL com a referência")
    parser.add_argument("--ref", default="alvo", help="campo de referência no manifest (padrão: alvo)")
    parser.add_argument(
        "--hyp", default="final", choices=("final", "raw"), help="texto avaliado (padrão: final)"
    )
    parser.add_argument("--json", type=Path, help="grava o relatório completo neste arquivo")
    parser.add_argument(
        "--baseline", type=Path, help="saída do harness da linha de base, para comparar (Δ e IC 95%%)"
    )
    parser.add_argument("--samples", type=int, default=2000, help="amostras do bootstrap (padrão: 2000)")
    parser.add_argument("--seed", type=int, default=0, help="semente do bootstrap (padrão: 0)")
    args = parser.parse_args(argv)

    try:
        report = build_report(args.results, args.manifest, args.ref, args.hyp)
        if args.baseline:
            baseline = build_report(args.baseline, args.manifest, args.ref, args.hyp)
            report["comparison"] = compare_reports(baseline, report, args.samples, args.seed)
    except InputError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    print(render(report))
    if "comparison" in report:
        print()
        print(render_comparison(report["comparison"]))
    if args.json:
        args.json.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
