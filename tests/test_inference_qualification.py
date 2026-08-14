from __future__ import annotations

import unittest
from dataclasses import replace
import json
from pathlib import Path
import tempfile

from albert_mvp.inference import LocalInferenceProfile
from albert_mvp.inference_qualification import (
    ContextProfilePlanner,
    ContextSource,
    DeterministicContextSelector,
    GOVERNED_FIXTURE_KINDS,
    InferenceQualificationService,
    PromotionError,
    QualificationReportStore,
    PromptPrefixReuseTracker,
    RuntimePin,
    build_governed_fixture_family,
    compare_context_profiles,
    default_context_profiles,
)


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
        "reviewed_latency_ms": 10,
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


def long_context_sources():
    return (ContextSource("long-context", "x" * 28_800, required=True),)


class GovernedFixtureFamilyTests(unittest.TestCase):
    def test_fixture_family_covers_every_required_profile_workload(self) -> None:
        fixtures = build_governed_fixture_family()

        self.assertEqual(
            tuple(fixture.kind for fixture in fixtures),
            GOVERNED_FIXTURE_KINDS,
        )
        self.assertEqual(len(fixtures), len(GOVERNED_FIXTURE_KINDS))
        self.assertEqual(
            len({fixture.fixture_id for fixture in fixtures}),
            len(fixtures),
        )
        self.assertTrue(all(fixture.prompt for fixture in fixtures))
        self.assertTrue(all(fixture.expected_outcomes for fixture in fixtures))

    def test_full_family_records_outcomes_for_each_governed_workload(self) -> None:
        profile = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1"),
            context_budget=16_384,
            output_budget=2_048,
        )
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )

        def run_fixture(fixture, _profile, _context, _repetition):
            result = {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 2,
                    "first_token_ms": 3,
                    "decoding_ms": 4,
                },
                "reviewed_latency_ms": 10,
            }
            result["runtime_pin"] = _context["runtime_pin"]
            if fixture.kind in {
                "small-edit",
                "multi-file-edit",
                "repair",
                "long-context",
                "queued-local-agent",
            }:
                result["route"] = {"kind": "coding-task"}
                result["plan"] = {"files": ["src/example.py"]}
                result["evidence"] = {"changed_files": ["src/example.py"]}
            if fixture.kind == "repair":
                result.update(outcome="repaired", repaired=True)
            elif fixture.kind == "malformed-output":
                result.update(outcome="escalated", escalated=True)
            elif fixture.kind == "policy-violation":
                result.update(outcome="policy-blocked", policy_blocked=True)
            elif fixture.kind == "cancellation":
                result.update(outcome="cancelled", cancelled=True)
            elif fixture.kind == "model-swap":
                result["model_swapped"] = True
                result["evidence"] = {"digest": "sha256:worker-digest"}
            elif fixture.kind == "queued-local-agent":
                result["queued"] = True
            result["model_digest"] = "sha256:worker-digest"
            return result

        report = InferenceQualificationService(
            Path("/tmp/qualification-family"), repetitions=2
        ).qualify(
            profile,
            run_fixture,
            runtime_pin=runtime_pin,
            rollback_test=successful_rollback_evidence,
            context_sources=long_context_sources(),
            required_source_ids=("long-context",),
        )

        self.assertEqual(report.metrics["sample_count"], 22)
        self.assertEqual(report.metrics["valid_routes"], 22)
        self.assertEqual(report.metrics["valid_plans"], 10)
        self.assertEqual(report.metrics["valid_evidence"], 12)
        self.assertEqual(report.metrics["accepted_outcomes"], 14)
        self.assertEqual(report.metrics["repairs"], 2)
        self.assertEqual(report.metrics["escalations"], 2)
        self.assertEqual(report.metrics["policy_blocks"], 2)
        self.assertEqual(report.metrics["cancellations"], 2)
        self.assertEqual(report.metrics["model_swaps"], 2)
        self.assertEqual(report.metrics["queued_local_agents"], 2)


class QualificationReportTests(unittest.TestCase):
    def test_repeated_fixture_runs_report_reviewed_quality_and_timings(self) -> None:
        fixture = next(
            item
            for item in build_governed_fixture_family()
            if item.kind == "small-edit"
        )
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

        def run_fixture(_fixture, _profile, _context, repetition):
            return {
                "route": {"kind": "coding-task"},
                "plan": {"files": ["src/example.py"]},
                "evidence": {
                    "changed_files": ["src/example.py"],
                    "tests": ["python -m unittest"],
                },
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "runtime_pin": _context["runtime_pin"],
                "timings": {
                    "load_ms": 12 + repetition,
                    "prompt_evaluation_ms": 20,
                    "first_token_ms": 3,
                    "decoding_ms": 18,
                },
                "reviewed_latency_ms": 140 + repetition,
            }

        with tempfile.TemporaryDirectory() as directory:
            report = InferenceQualificationService(
                Path(directory),
                fixtures=(fixture,),
                repetitions=2,
            ).qualify(
                profile,
                run_fixture,
                runtime_pin=runtime_pin,
                rollback_tested=True,
            )

        self.assertEqual(report.metrics["sample_count"], 2)
        self.assertEqual(report.metrics["valid_routes"], 2)
        self.assertEqual(report.metrics["valid_plans"], 2)
        self.assertEqual(report.metrics["valid_evidence"], 2)
        self.assertEqual(report.metrics["accepted_outcomes"], 2)
        self.assertEqual(report.metrics["repairs"], 0)
        self.assertEqual(report.metrics["escalations"], 0)
        self.assertEqual(report.metrics["reviewed_latency_ms"]["p50"], 140.0)
        self.assertEqual(report.metrics["timings_ms"]["load_ms"]["p95"], 13.0)
        self.assertFalse(report.promotion_ready)
        self.assertIn("fixture-coverage-incomplete", report.promotion_blockers)

    def test_invalid_runner_output_is_recorded_as_non_promotable_evidence(self) -> None:
        profile = LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1")
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        report = InferenceQualificationService(
            Path("/tmp/qualification-invalid"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=1,
        ).qualify(
            profile,
            lambda *_args: {"outcome": "not-a-real-outcome"},
            runtime_pin=runtime_pin,
            rollback_tested=True,
        )

        self.assertEqual(report.metrics["escalations"], 1)
        self.assertIn("reliability-not-qualified", report.promotion_blockers)

    def test_promotion_pins_report_runtime_and_replays_rollback(self) -> None:
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

        with tempfile.TemporaryDirectory() as directory:
            service = InferenceQualificationService(Path(directory), repetitions=2)
            baseline = service.qualify(
                profile,
                successful_fixture_result,
                runtime_pin=runtime_pin,
                rollback_test=baseline_rollback_evidence,
                context_sources=long_context_sources(),
                required_source_ids=("long-context",),
                report_id="baseline",
            )
            store = QualificationReportStore(Path(directory))
            store.save_report(baseline)
            report = service.qualify(
                profile,
                successful_fixture_result,
                runtime_pin=runtime_pin,
                rollback_test=baseline_rollback_evidence,
                baseline_report=baseline,
                context_sources=long_context_sources(),
                required_source_ids=("long-context",),
                report_id="candidate",
            )
            store.save_report(report)
            inspection = store.inspect_reports()
            promoted = store.promote(
                profile_id=profile.profile_id,
                report_id=report.report_id,
                runtime_pin=runtime_pin,
                correlation_id="promote-1",
                expected_revision=0,
            )
            self.assertEqual(promoted["active"]["report_id"], report.report_id)
            self.assertEqual(promoted["active"]["runtime_pin"], runtime_pin.to_dict())
            self.assertEqual(
                report.rollback_evidence["runtime_pin"], runtime_pin.to_dict()
            )
            self.assertEqual(
                report.rollback_receipt_id,
                report.rollback_evidence["receipt_id"],
            )
            self.assertIn(
                report.report_id,
                {item["report_id"] for item in inspection["reports"]},
            )
            self.assertNotIn("observations", inspection["reports"][0])

            replayed = store.promote(
                profile_id=profile.profile_id,
                report_id=report.report_id,
                runtime_pin=runtime_pin,
                correlation_id="promote-1",
                expected_revision=0,
            )
            self.assertEqual(replayed["last_action"], "promote-replay")
            rolled_back = store.rollback(
                profile.profile_id,
                correlation_id="rollback-1",
                expected_revision=1,
            )
            self.assertIsNone(rolled_back["active"])
            self.assertEqual(rolled_back["last_action"], "rollback")
            self.assertIsNone(store.inspect()["active"])
            rollback_replay = store.rollback(
                profile.profile_id,
                correlation_id="rollback-1",
                expected_revision=1,
            )
            self.assertEqual(rollback_replay["last_action"], "rollback-replay")
            state = store.inspect()
            state["revision"] = 32
            state["history"] = [
                {
                    "action": "promote",
                    "profile_id": profile.profile_id,
                    "report_id": report.report_id,
                    "correlation_id": f"historical-{index}",
                    "revision": index + 1,
                }
                for index in range(32)
            ]
            store._write_state(state)
            with self.assertRaisesRegex(
                PromotionError, "promotion history capacity exhausted"
            ):
                store.promote(
                    profile_id=profile.profile_id,
                    report_id=report.report_id,
                    runtime_pin=runtime_pin,
                    correlation_id="after-capacity",
                    expected_revision=32,
                )

    def test_runner_must_echo_the_qualified_runtime_identity(self) -> None:
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

        def runner(fixture, candidate, context, repetition):
            result = successful_fixture_result(fixture, candidate, context, repetition)
            result["runtime_pin"] = {**context["runtime_pin"], "runtime_id": "other"}
            return result

        report = InferenceQualificationService(
            Path("/tmp/qualification-runtime-identity"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=1,
        ).qualify(profile, runner, runtime_pin=runtime_pin)

        self.assertEqual(report.observations[0]["error_code"], "runner-failed")
        self.assertEqual(report.metrics["unexpected_outcomes"], 1)
        self.assertIn("reliability-not-qualified", report.promotion_blockers)

    def test_baseline_rejects_profile_configuration_drift(self) -> None:
        profile = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1"),
            model_digest="sha256:worker-digest",
        )
        changed_profile = replace(
            profile, context_budget=profile.context_budget + 1_024
        )
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        fixture = build_governed_fixture_family()[0]

        with tempfile.TemporaryDirectory() as directory:
            service = InferenceQualificationService(
                Path(directory), fixtures=(fixture,), repetitions=2
            )
            baseline = service.qualify(
                profile,
                successful_fixture_result,
                runtime_pin=runtime_pin,
                report_id="baseline",
            )
            service.report_store.save_report(baseline)
            candidate = service.qualify(
                changed_profile,
                successful_fixture_result,
                runtime_pin=runtime_pin,
                baseline_report=baseline,
                report_id="candidate",
            )

        self.assertIn("baseline-incomparable", candidate.promotion_blockers)

    def test_cancellation_after_runner_failure_is_recorded_as_cancellation(
        self,
    ) -> None:
        profile = LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1")
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        calls = 0

        def runner(*_args):
            nonlocal calls
            calls += 1
            raise RuntimeError("runner failed")

        report = InferenceQualificationService(
            Path("/tmp/qualification-cancel-after-failure"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=2,
        ).qualify(
            profile,
            runner,
            runtime_pin=runtime_pin,
            cancel_check=lambda: calls >= 1,
        )

        self.assertEqual(calls, 1)
        self.assertEqual(report.metrics["cancellations"], 2)
        self.assertTrue(
            all(
                item["error_code"] == "qualification-cancelled"
                for item in report.observations
            )
        )

    def test_expected_alternative_outcomes_preserve_fixture_quality(self) -> None:
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
        repair = next(
            fixture
            for fixture in build_governed_fixture_family()
            if fixture.kind == "repair"
        )
        policy = next(
            fixture
            for fixture in build_governed_fixture_family()
            if fixture.kind == "policy-violation"
        )

        def runner(fixture, _profile, context, _repetition):
            result = {
                "route": {"kind": "discussion"},
                "outcome": "escalated",
                "runtime_pin": context["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
            }
            if fixture.kind == "repair":
                result.update(
                    {
                        "route": {"kind": "coding-task"},
                        "plan": {"files": ["src/example.py"]},
                        "evidence": {"changed_files": ["src/example.py"]},
                        "outcome": "accepted",
                        "reviewed_latency_ms": 1,
                    }
                )
            return result

        repair_report = InferenceQualificationService(
            Path("/tmp/qualification-repair-alternative"),
            fixtures=(repair,),
            repetitions=1,
        ).qualify(profile, runner, runtime_pin=runtime_pin)
        policy_report = InferenceQualificationService(
            Path("/tmp/qualification-policy-alternative"),
            fixtures=(policy,),
            repetitions=1,
        ).qualify(profile, runner, runtime_pin=runtime_pin)

        self.assertTrue(repair_report.observations[0]["reliable"])
        self.assertTrue(policy_report.observations[0]["reliable"])
        self.assertEqual(repair_report.metrics["quality_rate"], 1.0)
        self.assertEqual(policy_report.metrics["quality_rate"], 1.0)

    def test_promotion_rejects_a_withdrawn_runtime(self) -> None:
        profile = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1"),
            model_digest="sha256:worker-digest",
        )
        withdrawn = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
            withdrawn=True,
        )

        def run_fixture(_fixture, _profile, _context, _repetition):
            return {
                "route": {"kind": "discussion"},
                "plan": {"files": []},
                "evidence": {"tests": ["not applicable"]},
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "runtime_pin": _context["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        with tempfile.TemporaryDirectory() as directory:
            report = InferenceQualificationService(
                Path(directory),
                fixtures=(build_governed_fixture_family()[0],),
                repetitions=1,
            ).qualify(
                profile,
                run_fixture,
                runtime_pin=withdrawn,
                rollback_tested=True,
            )
            store = QualificationReportStore(Path(directory))
            store.save_report(report)
            with self.assertRaises(PromotionError):
                store.promote(
                    profile_id=profile.profile_id,
                    report_id=report.report_id,
                    runtime_pin=withdrawn,
                    correlation_id="withdrawn-1",
                    expected_revision=0,
                )

    def test_report_persists_only_bounded_observation_metadata(self) -> None:
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

        def run_fixture(_fixture, _profile, _context, _repetition):
            return {
                "route": {"kind": "discussion"},
                "plan": {"private": "must not be persisted"},
                "evidence": {"private": "must not be persisted"},
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "runtime_pin": _context["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        with tempfile.TemporaryDirectory() as directory:
            report = InferenceQualificationService(
                Path(directory),
                fixtures=(build_governed_fixture_family()[0],),
                repetitions=1,
            ).qualify(
                profile,
                run_fixture,
                runtime_pin=runtime_pin,
                rollback_tested=True,
            )
            store = QualificationReportStore(Path(directory))
            store.save_report(report)
            restored = store.load_report(report.report_id)

        self.assertEqual(restored.report_id, report.report_id)
        self.assertNotIn("prompt", restored.to_dict())
        self.assertNotIn("raw_output", restored.to_dict())
        self.assertNotIn("plan", restored.to_dict()["observations"][0])
        self.assertNotIn("evidence", restored.to_dict()["observations"][0])

    def test_persisted_report_rejects_tampered_metrics_and_blockers(self) -> None:
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

        def run_fixture(_fixture, _profile, _context, _repetition):
            return {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "runtime_pin": _context["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        with tempfile.TemporaryDirectory() as directory:
            report = InferenceQualificationService(
                Path(directory),
                fixtures=(build_governed_fixture_family()[0],),
                repetitions=1,
            ).qualify(
                profile,
                run_fixture,
                runtime_pin=runtime_pin,
                rollback_tested=True,
            )
            store = QualificationReportStore(Path(directory))
            store.save_report(report)
            report_path = next(
                (Path(directory) / "inference/qualification/reports").glob("*.json")
            )
            payload = json.loads(report_path.read_text(encoding="utf-8"))

            payload["metrics"]["quality_rate"] = 0.5
            report_path.write_text(json.dumps(payload), encoding="utf-8")
            with self.assertRaises(PromotionError):
                store.load_report(report.report_id)

            payload = report.to_dict()
            payload["promotion_ready"] = True
            payload["promotion_blockers"] = ["quality-not-qualified"]
            report_path.write_text(json.dumps(payload), encoding="utf-8")
            with self.assertRaises(PromotionError):
                store.load_report(report.report_id)

    def test_fixture_definition_and_repetition_integrity_are_persisted(self) -> None:
        profile = LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1")
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        original = build_governed_fixture_family()[0]
        substituted = replace(original, prompt="substituted workload")

        with tempfile.TemporaryDirectory() as directory:
            report = InferenceQualificationService(
                Path(directory), fixtures=(substituted,), repetitions=2
            ).qualify(
                profile,
                lambda *_args: {
                    "route": {"kind": "discussion"},
                    "outcome": "accepted",
                    "runtime_pin": _args[2]["runtime_pin"],
                    "timings": {
                        "load_ms": 1,
                        "prompt_evaluation_ms": 1,
                        "first_token_ms": 1,
                        "decoding_ms": 1,
                    },
                    "reviewed_latency_ms": 1,
                },
                runtime_pin=runtime_pin,
            )
            store = QualificationReportStore(Path(directory))
            with self.assertRaises(ValueError):
                store.save_report(report)

        with tempfile.TemporaryDirectory() as directory:
            report = InferenceQualificationService(
                Path(directory), fixtures=(original,), repetitions=2
            ).qualify(
                profile,
                lambda *_args: {
                    "route": {"kind": "discussion"},
                    "outcome": "accepted",
                    "runtime_pin": _args[2]["runtime_pin"],
                    "timings": {
                        "load_ms": 1,
                        "prompt_evaluation_ms": 1,
                        "first_token_ms": 1,
                        "decoding_ms": 1,
                    },
                    "reviewed_latency_ms": 1,
                },
                runtime_pin=runtime_pin,
            )
            store = QualificationReportStore(Path(directory))
            store.save_report(report)
            path = next(
                (Path(directory) / "inference/qualification/reports").glob("*.json")
            )
            payload = json.loads(path.read_text(encoding="utf-8"))
            payload["observations"][1]["repetition"] = 0
            path.write_text(json.dumps(payload), encoding="utf-8")
            with self.assertRaises(PromotionError):
                store.load_report(report.report_id)

    def test_profile_schema_names_do_not_trigger_observation_truth_filter(self) -> None:
        profile = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1"),
            schema={"properties": {"plan": {"type": "string"}}},
        )
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        report = InferenceQualificationService(
            Path("/tmp/qualification-schema"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=1,
        ).qualify(
            profile,
            lambda *_args: {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "runtime_pin": _args[2]["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            },
            runtime_pin=runtime_pin,
        )
        with tempfile.TemporaryDirectory() as directory:
            QualificationReportStore(Path(directory)).save_report(report)

    def test_qualification_can_stop_cooperatively_without_claiming_promotion(
        self,
    ) -> None:
        profile = LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1")
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        calls = 0

        def runner(*_args):
            nonlocal calls
            calls += 1
            context = _args[2]
            self.assertIn("cancel_check", context)
            self.assertIn("deadline_at", context)
            return {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "runtime_pin": context["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        report = InferenceQualificationService(
            Path("/tmp/qualification-cancel"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=3,
        ).qualify(
            profile,
            runner,
            runtime_pin=runtime_pin,
            cancel_check=lambda: calls >= 1,
        )

        self.assertEqual(calls, 1)
        self.assertEqual(report.metrics["cancellations"], 3)
        self.assertIn("unexpected-outcome", report.promotion_blockers)

    def test_rollback_flag_alone_cannot_qualify_without_executed_receipt(self) -> None:
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
        runner = successful_fixture_result
        with tempfile.TemporaryDirectory() as directory:
            service = InferenceQualificationService(
                Path(directory),
                fixtures=(build_governed_fixture_family()[0],),
                repetitions=2,
            )
            unproven = service.qualify(
                profile,
                runner,
                runtime_pin=runtime_pin,
                rollback_tested=True,
            )
            previous = service.qualify(
                profile,
                runner,
                runtime_pin=runtime_pin,
                report_id="previous-report",
            )
            service.report_store.save_report(previous)
            proven = service.qualify(
                profile,
                runner,
                runtime_pin=runtime_pin,
                rollback_test=successful_rollback_evidence,
            )

        self.assertIn("rollback-not-tested", unproven.promotion_blockers)
        self.assertEqual(unproven.rollback_receipt_id, "")
        self.assertTrue(proven.rollback_receipt_id.startswith("rollback-test:"))
        self.assertNotIn("rollback-not-tested", proven.promotion_blockers)

    def test_persisted_candidate_requires_an_existing_baseline_report(self) -> None:
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
        service = InferenceQualificationService(
            Path("/tmp/qualification-baseline-proof"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=2,
        )
        baseline = service.qualify(
            profile,
            successful_fixture_result,
            runtime_pin=runtime_pin,
            rollback_test=successful_rollback_evidence,
            report_id="missing-baseline",
        )
        candidate = service.qualify(
            profile,
            successful_fixture_result,
            runtime_pin=runtime_pin,
            rollback_test=baseline_rollback_evidence,
            baseline_report=baseline,
            report_id="candidate-with-missing-baseline",
        )

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(PromotionError):
                QualificationReportStore(Path(directory)).save_report(candidate)

    def test_unqualified_profile_and_oversized_context_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            ContextSource("too-large", "x" * (1_048_576 + 1))
        profile = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-v1"),
            qualified=False,
        )
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        report = InferenceQualificationService(
            Path("/tmp/qualification-unqualified"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=1,
        ).qualify(
            profile,
            lambda *_args: {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "runtime_pin": _args[2]["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            },
            runtime_pin=runtime_pin,
        )
        self.assertIn("profile-not-qualified", report.promotion_blockers)

    def test_promotion_requires_non_inferior_quality_and_reliability(self) -> None:
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
        fixture = next(
            item
            for item in build_governed_fixture_family()
            if item.kind == "small-edit"
        )

        def run_result(_route_valid: bool):
            return lambda *_args: {
                "route": {"kind": "coding-task"} if _route_valid else {},
                "plan": {"files": ["src/example.py"]},
                "evidence": {"changed_files": ["src/example.py"]},
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "runtime_pin": _args[2]["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        with tempfile.TemporaryDirectory() as directory:
            service = InferenceQualificationService(
                Path(directory), fixtures=(fixture,), repetitions=1
            )
            baseline = service.qualify(
                profile,
                run_result(True),
                runtime_pin=runtime_pin,
                rollback_tested=True,
                report_id="baseline",
            )
            regression = service.qualify(
                profile,
                run_result(False),
                runtime_pin=runtime_pin,
                rollback_tested=True,
                baseline_report=baseline,
                report_id="regression",
            )

        self.assertIn("quality-regression", regression.promotion_blockers)
        self.assertIn("quality-not-qualified", regression.promotion_blockers)

    def test_context_measurements_are_reported_without_caching_outcomes(self) -> None:
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
        calls: list[int] = []

        def run_fixture(_fixture, _profile, context, repetition):
            calls.append(repetition)
            self.assertIn("source_digest", context)
            return {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "runtime_pin": context["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        service = InferenceQualificationService(
            Path("/tmp/qualification-context"),
            fixtures=(build_governed_fixture_family()[0],),
            repetitions=2,
        )
        report = service.qualify(
            profile,
            run_fixture,
            runtime_pin=runtime_pin,
            rollback_tested=True,
            context_sources=(
                ContextSource("mission", "stable mission context", required=True),
            ),
            required_source_ids=("mission",),
        )

        self.assertEqual(calls, [0, 1])
        self.assertEqual(report.metrics["context_cache_hits"], 1)
        self.assertEqual(report.metrics["prefix_reuses"], 1)
        self.assertEqual(report.metrics["prefix_invalidations"], 0)


class ContextQualificationTests(unittest.TestCase):
    def test_default_controller_and_worker_profiles_use_bounded_v1_sizes(self) -> None:
        profiles = default_context_profiles("gemma4:12b")

        self.assertEqual(profiles["controller"]["initial"].context_budget, 8_192)
        self.assertEqual(profiles["controller"]["expanded"].context_budget, 16_384)
        self.assertEqual(profiles["normal-worker"]["initial"].context_budget, 16_384)
        self.assertEqual(profiles["normal-worker"]["expanded"].context_budget, 32_768)

    def test_context_selection_is_digest_keyed_and_prefix_reuse_is_exact(self) -> None:
        selector = DeterministicContextSelector()
        original = (
            ContextSource("required", "Always include this source.", required=True),
            ContextSource("optional", "Include this stable source when it fits."),
        )

        first = selector.select(
            original,
            profile_role="controller",
            budget_tokens=64,
            required_source_ids=("required",),
        )
        second = selector.select(
            original,
            profile_role="controller",
            budget_tokens=64,
            required_source_ids=("required",),
        )
        changed = selector.select(
            (
                original[0],
                ContextSource(
                    "optional", "The source changed and must invalidate reuse."
                ),
            ),
            profile_role="controller",
            budget_tokens=64,
            required_source_ids=("required",),
        )

        self.assertFalse(first.cache_hit)
        self.assertTrue(second.cache_hit)
        self.assertEqual(first.source_digest, second.source_digest)
        self.assertNotEqual(first.source_digest, changed.source_digest)

        tracker = PromptPrefixReuseTracker()
        self.assertFalse(tracker.observe(first.prompt_prefix).reused)
        self.assertTrue(tracker.observe(first.prompt_prefix).reused)
        invalidated = tracker.observe(changed.prompt_prefix)
        self.assertTrue(invalidated.invalidated)

    def test_context_prefix_reuse_ignores_fixture_suffix_changes(self) -> None:
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
        fixtures = build_governed_fixture_family()[:2]

        def runner(fixture, _profile, context, _repetition):
            return {
                "route": {"kind": "discussion"},
                "outcome": "accepted",
                "model_digest": "sha256:worker-digest",
                "runtime_pin": context["runtime_pin"],
                "timings": {
                    "load_ms": 1,
                    "prompt_evaluation_ms": 1,
                    "first_token_ms": 1,
                    "decoding_ms": 1,
                },
                "reviewed_latency_ms": 1,
            }

        report = InferenceQualificationService(
            Path("/tmp/qualification-prefix-suffix"),
            fixtures=fixtures,
            repetitions=1,
        ).qualify(
            profile,
            runner,
            runtime_pin=runtime_pin,
            context_sources=(ContextSource("mission", "stable mission context"),),
            required_source_ids=("mission",),
        )

        self.assertEqual(report.metrics["prefix_reuses"], 1)
        self.assertEqual(report.metrics["prefix_invalidations"], 0)

    def test_context_expands_only_when_required_material_fits_and_quality_improves(
        self,
    ) -> None:
        initial = replace(
            LocalInferenceProfile.default("gemma4:12b", profile_id="worker-initial"),
            context_budget=8_192,
            output_budget=1_024,
        )
        expanded = replace(
            initial,
            profile_id="worker-expanded",
            context_budget=16_384,
        )

        decision = ContextProfilePlanner.choose(
            initial,
            expanded,
            required_context_tokens=9_000,
            initial_quality=0.75,
            expanded_quality=1.0,
            initial_reliability=0.8,
            expanded_reliability=1.0,
        )

        self.assertTrue(decision.expanded)
        self.assertEqual(decision.profile.profile_id, "worker-expanded")
        comparison = compare_context_profiles(
            "normal-worker",
            initial,
            expanded,
            required_context_tokens=9_000,
            initial_quality=0.75,
            expanded_quality=1.0,
            initial_reliability=0.8,
            expanded_reliability=1.0,
        )
        self.assertTrue(comparison.to_dict()["expanded"])
        self.assertEqual(comparison.to_dict()["selected_profile_id"], "worker-expanded")

    def test_qualification_rejects_long_context_before_runner_when_headroom_is_insufficient(
        self,
    ) -> None:
        fixture = next(
            item
            for item in build_governed_fixture_family()
            if item.kind == "long-context"
        )
        profile = LocalInferenceProfile.default("qwen3:14b", profile_id="controller-v1")
        runtime_pin = RuntimePin(
            runtime_id="ollama",
            runtime_version="0.11.4",
            binary_digest="a" * 64,
            configuration_digest="b" * 64,
        )
        called = False

        def runner(*_args):
            nonlocal called
            called = True
            return {}

        report = InferenceQualificationService(
            Path("/tmp/qualification-headroom"), fixtures=(fixture,), repetitions=1
        ).qualify(
            profile,
            runner,
            runtime_pin=runtime_pin,
            rollback_tested=True,
        )

        self.assertFalse(called)
        self.assertEqual(report.metrics["context_over_budget"], 1)
        self.assertIn("unexpected-outcome", report.promotion_blockers)


if __name__ == "__main__":
    unittest.main()
