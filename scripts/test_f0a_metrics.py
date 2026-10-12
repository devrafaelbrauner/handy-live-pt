"""Testes de scripts/f0a_metrics.py.

    python3 -m unittest discover -s scripts -p "test_*.py"
"""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from f0a_metrics import (  # noqa: E402
    InputError,
    align,
    build_report,
    edit_distance,
    itn_spans,
    main,
    normalized_words,
    punctuation_counts,
    words_with_marks,
)


class TextTests(unittest.TestCase):
    def test_normalized_words_drop_case_and_punctuation_but_keep_accents_and_numbers(self):
        ref = normalized_words("Oi, Carla! Às 18h30, no 3º andar.")
        self.assertEqual(ref, ["oi", "carla", "às", "18h30", "no", "3º", "andar"])
        self.assertEqual(edit_distance(ref, normalized_words("oi carla às 18h30 no 3º andar")), 0)
        self.assertEqual(edit_distance(ref, normalized_words("oi carla as 18h30 no 3º andar")), 1)

    def test_marks_follow_their_word_with_priority_for_question_and_ellipsis(self):
        self.assertEqual(
            words_with_marks("Sério?! Bom... Então: tá"),
            [("sério", "?"), ("bom", "."), ("então", ":"), ("tá", None)],
        )

    def test_itn_spans_cover_brazilian_number_formats_but_not_digits_inside_words(self):
        self.assertEqual(
            itn_spans("Às 18h30, paguei R$ 15.000,00 (10%) no dia 1º, 3,5 vezes às 18:30 em mp3."),
            ["18h30", "R$ 15.000,00", "10%", "1º", "3,5", "18:30"],
        )


class AlignmentTests(unittest.TestCase):
    def test_alignment_is_minimal_and_pairs_substitutions_on_the_diagonal(self):
        distance, pairs = align(list("abcd"), list("axc"))
        self.assertEqual(distance, 2)
        self.assertEqual(pairs, [(0, 0), (1, 1), (2, 2), (3, None)])
        self.assertEqual(edit_distance(list("abcd"), list("axc")), 2)

    def test_punctuation_is_scored_on_aligned_words(self):
        counts = punctuation_counts("Oi, Carla. Tudo bem?", "Oi Carla, tudo bem?")
        self.assertEqual(dict(counts[","]), {"fp": 1, "fn": 1})
        self.assertEqual(dict(counts["."]), {"fn": 1})
        self.assertEqual(dict(counts["?"]), {"tp": 1})


class ReportTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)
        self.manifest = self._jsonl(
            "m.jsonl",
            [
                {"id": "a", "audio": "a.wav", "alvo": "Oi, Carla. Às 18h30."},
                {"id": "s", "audio": "s.wav", "alvo": ""},
                {"audio": "c.wav", "alvo": "Sim."},
                {"id": "m", "audio": "m.wav", "alvo": "Faltou."},
            ],
        )

    def _jsonl(self, name, rows):
        path = self.dir / name
        path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        return path

    def test_report_joins_by_id_and_separates_probes_failures_and_missing(self):
        results = self._jsonl(
            "r.jsonl",
            [
                {"type": "utterance", "id": "a", "audio_secs": 12.0,
                 "raw": "é oi carla às 18:30", "final": "Oi carla às 18:30."},
                {"type": "utterance", "id": "s", "audio_secs": 2.0, "raw": "obrigado", "final": "Obrigado."},
                {"type": "utterance", "id": "c.wav", "error": "transcribe failed: boom"},
                {"type": "summary", "model": "x", "vad": "silero", "language": "pt"},
            ],
        )

        report = build_report(results, self.manifest, "alvo", "final")

        self.assertEqual(
            report["counts"],
            {"scored": 1, "silence_probes": 1, "failed": ["c.wav"], "missing": ["m"]},
        )
        self.assertEqual(report["rates"]["all"]["wer"], 0.25)
        self.assertEqual(report["rates"]["10–30 s"]["wer"], 0.25)
        self.assertIsNone(report["rates"]["≤ 10 s"]["wer"])
        self.assertEqual(report["itn"], {"ref_spans": 1, "matched": 0, "exact_match": 0.0, "spurious": 1})
        self.assertEqual(
            {mark: (row["tp"], row["fp"], row["fn"]) for mark, row in report["punctuation"].items()},
            {",": (0, 0, 1), ".": (1, 0, 1), "?": (0, 0, 0), "!": (0, 0, 0), ":": (0, 0, 0),
             "all": (1, 0, 2)},
        )
        self.assertEqual(
            report["preservation"],
            {"raw_words": 6, "edits": 1, "rate": 1 / 6, "utterances_changed": 1},
        )
        self.assertEqual(report["silence"], {"probes": 1, "inserted_words": 1, "probes_with_words": 1})

    def test_results_with_ids_outside_the_manifest_are_rejected(self):
        results = self._jsonl(
            "r.jsonl",
            [{"type": "utterance", "id": "zzz", "audio_secs": 1.0, "raw": "x", "final": "x"}],
        )
        with self.assertRaisesRegex(InputError, "ids not in"):
            build_report(results, self.manifest, "alvo", "final")
        with contextlib.redirect_stderr(io.StringIO()) as stderr:
            self.assertEqual(main([str(results), "--manifest", str(self.manifest)]), 2)
        self.assertIn("zzz", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
