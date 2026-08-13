from __future__ import annotations

from dataclasses import replace
from io import StringIO
from pathlib import Path
import json
import tempfile
import unittest
from unittest.mock import patch

from albert_mvp.cli import main
from albert_mvp.inference import LocalInferenceProfile
from albert_mvp.inference_qualification import (
    InferenceQualificationService,
    QualificationReportStore,
    RuntimePin,
    build_governed_fixture_family,
)
from albert_mvp.server import serve


class InferenceQualificationCliTests(unittest.TestCase):
    def make_report(self, runtime: Path):
        profile = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1"),
            model_digest="sha256:worker-digest",
        )
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )

        def runner(_fixture, _profile, _context, _repetition):
            return {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        report = InferenceQualificationService(
            runtime,
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=1,
        ).qualify(
            profile,
            runner,
            runtime_pin=runtime_pin,
            rollback_tested=True,
        )
        QualificationReportStore(runtime).save_report(report)
        return report, runtime_pin

    def common(self, runtime: Path) -> list[str]:
        return [
            "--target-repo",
            str(runtime / "target"),
            "--runtime-root",
            str(runtime),
        ]

    def test_cli_inspect_promote_and_rollback_share_the_report_store_boundary(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory)
            (runtime / "target").mkdir()
            report, runtime_pin = self.make_report(runtime)
            captured: list[str] = []

            def capture(value: str = "", *args, **kwargs):
                captured.append(value)

            with patch("builtins.print", side_effect=capture):
                self.assertEqual(
                    main(["inference-qualification", *self.common(runtime)]),
                    0,
                )
            inspection = json.loads(captured[-1])
            self.assertEqual(inspection["reports"][0]["report_id"], report.report_id)

            with patch("builtins.print", side_effect=capture):
                self.assertEqual(
                    main(
                        [
                            "inference-qualification-promote",
                            *self.common(runtime),
                            "--report-id",
                            report.report_id,
                            "--profile-id",
                            "worker-v1",
                            "--runtime-id",
                            runtime_pin.runtime_id,
                            "--runtime-version",
                            runtime_pin.runtime_version,
                            "--binary-digest",
                            runtime_pin.binary_digest,
                            "--configuration-digest",
                            runtime_pin.configuration_digest,
                        ]
                    ),
                    0,
                )
            promoted = json.loads(captured[-1])
            self.assertEqual(promoted["active"]["report_id"], report.report_id)

            with patch("builtins.print", side_effect=capture):
                self.assertEqual(
                    main(
                        [
                            "inference-qualification-rollback",
                            *self.common(runtime),
                            "--profile-id",
                            "worker-v1",
                        ]
                    ),
                    0,
                )
            rolled_back = json.loads(captured[-1])
            self.assertIsNone(rolled_back["active"])

    def test_persistent_transport_returns_the_same_inspection_projection(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory)
            (runtime / "target").mkdir()
            report, _runtime_pin = self.make_report(runtime)
            argv = ["inference-qualification", *self.common(runtime)]
            output = StringIO()
            serve(
                StringIO(json.dumps({"id": "qualification-1", "argv": argv}) + "\n"),
                output,
            )

        response = json.loads(output.getvalue())
        self.assertEqual(response["id"], "qualification-1")
        self.assertTrue(response["success"])
        projection = json.loads(response["stdout"])
        self.assertEqual(projection["reports"][0]["report_id"], report.report_id)


if __name__ == "__main__":
    unittest.main()
