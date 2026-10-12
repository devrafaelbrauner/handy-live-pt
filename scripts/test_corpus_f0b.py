"""Testes do export de scripts/corpus_f0b.py.

    python3 -m unittest discover -s scripts -p "test_*.py"
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from corpus_f0b import export_rows  # noqa: E402

MANIFEST = [
    {
        "id": "b01-f01", "bloco": "b01", "tags": ["numeros"],
        "leia": "Oi, Carla! Você consegue chegar às dezoito e trinta amanhã?",
        "alvo": "Oi, Carla! Você consegue chegar às 18h30 amanhã?",
    },
    {
        "id": "b01-f02", "bloco": "b01", "tags": [],
        "leia": "Confirmado o jantar de sábado com a família toda reunida.",
        "alvo": "Confirmado o jantar de sábado com a família toda reunida.",
    },
    {"id": "b01-s01", "bloco": "b01", "tags": ["silencio"], "leia": "", "alvo": ""},
    {"id": "b01-s02", "bloco": "b01", "tags": ["silencio"], "leia": "", "alvo": ""},
    {
        "id": "b02-f01", "bloco": "b02", "tags": [],
        "leia": "A reunião de amanhã foi adiada para quinta-feira à tarde.",
        "alvo": "A reunião de amanhã foi adiada para quinta-feira à tarde.",
    },
]

HISTORY = [
    (1000, "handy-1000.wav", "oi carla você consegue chegar às 18h30 amanhã"),
    (1100, "handy-1100.wav", "Oi, Carla! Você consegue chegar às 18:30 amanhã?"),
    (1200, "handy-1200.wav", "Confirmado o jantar de sábado com a família toda reunida."),
    (1250, "handy-1250.wav", ""),
    (1300, "handy-1300.wav", ""),
    (1400, "handy-1400.wav", "Obrigado."),
    (9999, "handy-9999.wav", ""),
]


class ExportTests(unittest.TestCase):
    def test_export_picks_latest_existing_takes_and_fills_probes_per_block(self):
        with tempfile.TemporaryDirectory() as tmp:
            rec_dir = Path(tmp)
            for _, name, _ in HISTORY:
                if name != "handy-1200.wav":  # deleted by the retention policy
                    (rec_dir / name).write_bytes(b"")

            rows, report = export_rows(HISTORY, MANIFEST, rec_dir, 0.55, 600)

            exported = {row["id"]: (Path(row["audio"]).name, row["texto_app"]) for row in rows}
            self.assertEqual(
                exported,
                {
                    "b01-f01": ("handy-1100.wav", "Oi, Carla! Você consegue chegar às 18:30 amanhã?"),
                    # Three short recordings near b01 for two probes: the two most recent win.
                    "b01-s01": ("handy-1300.wav", ""),
                    "b01-s02": ("handy-1400.wav", "Obrigado."),
                },
            )
            self.assertEqual(
                report,
                {"missing": ["b02-f01"], "lost": ["b01-f02"], "unassigned_short": 2},
            )
            self.assertTrue(all(Path(row["audio"]).is_absolute() for row in rows))
            self.assertEqual(rows[0]["alvo"], MANIFEST[0]["alvo"])


if __name__ == "__main__":
    unittest.main()
