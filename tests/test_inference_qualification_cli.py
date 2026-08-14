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
    ContextSource,
    InferenceQualificationService,
    QualificationReportStore,
    RuntimePin,
)
from albert_mvp.server import serve


def successful_fixture_result(fixture, _profile, context, _repetition):
    edit_kinds = {
        "small-edit",
        "multi-file-edit",
        "repair",
        "long-context",
        "queued-local-agent",
    }
    evidence_kinds = edit_kinds | {"model-swap"}
    result = {
        "route": {
            "kind": "coding-task" if fixture.kind in edit_kinds else "discussion"
        },
        "outcome": "accepted",
        "model_digest": "sha256:worker-digest",
        "runtime_pin": context["runtime_pin"],
        "timings": {
            "load_ms": 1,
            "prompt_evaluation_ms": 2,
            "first_token_ms": 3,
            "decoding_ms": 4,
        },
        "reviewed_latency_ms": 1,
    }
    if fixture.kind in edit_kinds:
        result["plan"] = {"files": ["src/example.py"]}
    if fixture.kind in evidence_kinds:
        result["evidence"] = {"changed_files": ["src/example.py"]}
    if fixture.kind == "repair":
        result["outcome"] = "repaired"
    elif fixture.kind == "malformed-output":
        result["outcome"] = "escalated"
    elif fixture.kind == "policy-violation":
        result["outcome"] = "policy-blocked"
    elif fixture.kind == "cancellation":
        result["outcome"] = "cancelled"
    elif fixture.kind == "model-swap":
        result["model_swapped"] = True
    elif fixture.kind == "queued-local-agent":
        result["queued"] = True
    return result


def successful_rollback_evidence(profile, runtime_pin, _control):
    return {
        "profile_id": profile.profile_id,
        "runtime_pin": runtime_pin.to_dict(),
        "previous_report_id": "previous-report",
        "restored_report_id": "previous-report",
        "replay_verified": True,
    }


def baseline_rollback_evidence(profile, runtime_pin, _control):
    return {
        "profile_id": profile.profile_id,
        "runtime_pin": runtime_pin.to_dict(),
        "previous_report_id": "baseline",
        "restored_report_id": "baseline",
        "replay_verified": True,
    }


class InferenceQualificationCliTests(unittest.TestCase):
    def make_report(self, runtime: Path):
        profile = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1"),
            model_digest="sha256:worker-digest",
            context_budget=16_384,
            output_budget=2_048,
        )
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )

        service = InferenceQualificationService(runtime, repetitions=2)
        baseline = service.qualify(
            profile,
            successful_fixture_result,
            runtime_pin=runtime_pin,
            rollback_test=successful_rollback_evidence,
            context_sources=(
                ContextSource("long-context", "x" * 28_800, required=True),
            ),
            required_source_ids=("long-context",),
            report_id="baseline",
        )
        QualificationReportStore(runtime).save_report(baseline)
        report = service.qualify(
            profile,
            successful_fixture_result,
            runtime_pin=runtime_pin,
            rollback_test=baseline_rollback_evidence,
            baseline_report=baseline,
            context_sources=(
                ContextSource("long-context", "x" * 28_800, required=True),
            ),
            required_source_ids=("long-context",),
            report_id="candidate",
        )
        store = QualificationReportStore(runtime)
        store.save_report(report)
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
            self.assertIn(
                report.report_id,
                {item["report_id"] for item in inspection["reports"]},
            )

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
                            "--correlation-id",
                            "promote-1",
                            "--expected-revision",
                            "0",
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
                            "--correlation-id",
                            "rollback-1",
                            "--expected-revision",
                            "1",
                        ]
                    ),
                    0,
                )
            rolled_back = json.loads(captured[-1])
            self.assertEqual(rolled_back["active"]["report_id"], "baseline")

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
        self.assertIn(
            report.report_id,
            {item["report_id"] for item in projection["reports"]},
        )

    def test_cli_projects_malformed_promotion_state_as_structured_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory)
            (runtime / "target").mkdir()
            state_path = runtime / "inference/qualification/promotion-state.json"
            state_path.parent.mkdir(parents=True)
            state_path.write_text("{}", encoding="utf-8")
            stderr = StringIO()

            with patch("sys.stderr", stderr):
                result = main(["inference-qualification", *self.common(runtime)])

        self.assertEqual(result, 1)
        error = json.loads(stderr.getvalue())
        self.assertEqual(error["error"]["code"], "inference-qualification-failed")
        self.assertTrue(error["error"]["recoverable"])

    def test_persistent_transport_replays_promotion_and_rollback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory)
            (runtime / "target").mkdir()
            report, runtime_pin = self.make_report(runtime)
            promote_argv = [
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
                "--correlation-id",
                "promote-transport-1",
                "--expected-revision",
                "0",
            ]
            rollback_argv = [
                "inference-qualification-rollback",
                *self.common(runtime),
                "--profile-id",
                "worker-v1",
                "--correlation-id",
                "rollback-transport-1",
                "--expected-revision",
                "1",
            ]
            request = (
                "\n".join(
                    json.dumps({"id": str(index), "argv": argv})
                    for index, argv in enumerate(
                        (promote_argv, promote_argv, rollback_argv, rollback_argv), 1
                    )
                )
                + "\n"
            )
            output = StringIO()
            serve(StringIO(request), output)

        responses = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertTrue(all(response["success"] for response in responses))
        self.assertEqual(json.loads(responses[1]["stdout"])["last_action"], "promote")
        self.assertEqual(json.loads(responses[3]["stdout"])["last_action"], "rollback")


if __name__ == "__main__":
    unittest.main()
