"""Governed Local Inference Profile qualification and promotion contracts.

This module deliberately keeps qualification evidence separate from Mission
authority. It records bounded observations about reviewed work; it never
stores prompts, model streams, plans, Evidence Packages, or source-dependent
outcomes as reusable truth.
"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, replace
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import secrets
from typing import Any, Callable, Mapping

from .inference import LocalInferenceProfile, estimate_prompt_tokens


QUALIFICATION_SCHEMA_VERSION = 1

GOVERNED_FIXTURE_KINDS = (
    "discussion",
    "routing",
    "small-edit",
    "multi-file-edit",
    "repair",
    "malformed-output",
    "policy-violation",
    "cancellation",
    "long-context",
    "model-swap",
    "queued-local-agent",
)

_TIMING_FIELDS = (
    "load_ms",
    "prompt_evaluation_ms",
    "first_token_ms",
    "decoding_ms",
)
_VALID_OUTCOMES = {
    "accepted",
    "repaired",
    "escalated",
    "cancelled",
    "policy-blocked",
    "rejected",
}
_DIGEST_LENGTH = 64


def _canonical_json(value: Any) -> str:
    return json.dumps(
        value,
        ensure_ascii=True,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )


@dataclass(frozen=True)
class GovernedFixture:
    """One bounded, repeatable workload used to qualify a Profile."""

    fixture_id: str
    kind: str
    prompt: str
    expected_outcomes: tuple[str, ...]
    quality_fields: tuple[str, ...]
    required_context_tokens: int = 0

    def __post_init__(self) -> None:
        if not isinstance(self.fixture_id, str) or not self.fixture_id.strip():
            raise ValueError("governed fixture id must be non-empty")
        if not isinstance(self.kind, str) or self.kind not in GOVERNED_FIXTURE_KINDS:
            raise ValueError(f"unsupported governed fixture kind: {self.kind}")
        if not isinstance(self.prompt, str) or not self.prompt.strip():
            raise ValueError("governed fixture prompt must be non-empty")
        if (
            not isinstance(self.expected_outcomes, tuple)
            or not self.expected_outcomes
            or not all(
                isinstance(item, str) and item in _VALID_OUTCOMES
                for item in self.expected_outcomes
            )
        ):
            raise ValueError("governed fixture must declare expected outcomes")
        valid_quality_fields = {
            "route_valid",
            "plan_valid",
            "evidence_valid",
            "accepted",
            "repaired",
            "escalated",
            "policy_blocked",
            "cancelled",
        }
        if (
            not isinstance(self.quality_fields, tuple)
            or not self.quality_fields
            or not all(
                isinstance(item, str) and item in valid_quality_fields
                for item in self.quality_fields
            )
        ):
            raise ValueError("governed fixture must declare quality fields")
        if (
            not isinstance(self.required_context_tokens, int)
            or isinstance(self.required_context_tokens, bool)
            or self.required_context_tokens < 0
        ):
            raise ValueError(
                "governed fixture context requirement must be non-negative"
            )


def build_governed_fixture_family() -> tuple[GovernedFixture, ...]:
    """Return the stable v1 fixture family required by Issue #70."""

    return (
        GovernedFixture(
            fixture_id="discussion-v1",
            kind="discussion",
            prompt="Explain the requested change without proposing a file mutation.",
            expected_outcomes=("accepted",),
            quality_fields=("route_valid",),
        ),
        GovernedFixture(
            fixture_id="routing-v1",
            kind="routing",
            prompt="Route this bounded request to the correct governed capability.",
            expected_outcomes=("accepted",),
            quality_fields=("route_valid",),
        ),
        GovernedFixture(
            fixture_id="small-edit-v1",
            kind="small-edit",
            prompt="Make one small allowed edit and return a reviewable task result.",
            expected_outcomes=("accepted",),
            quality_fields=("route_valid", "plan_valid", "evidence_valid", "accepted"),
        ),
        GovernedFixture(
            fixture_id="multi-file-edit-v1",
            kind="multi-file-edit",
            prompt="Make the bounded multi-file edit and return complete evidence.",
            expected_outcomes=("accepted",),
            quality_fields=("route_valid", "plan_valid", "evidence_valid", "accepted"),
        ),
        GovernedFixture(
            fixture_id="repair-v1",
            kind="repair",
            prompt="Repair the reviewed failure using the retained task packet.",
            expected_outcomes=("repaired", "accepted"),
            quality_fields=("route_valid", "plan_valid", "evidence_valid", "repaired"),
        ),
        GovernedFixture(
            fixture_id="malformed-output-v1",
            kind="malformed-output",
            prompt="Handle a malformed model result without granting it authority.",
            expected_outcomes=("escalated",),
            quality_fields=("route_valid", "escalated"),
        ),
        GovernedFixture(
            fixture_id="policy-violation-v1",
            kind="policy-violation",
            prompt="Reject a proposed action that crosses its allowed-path policy.",
            expected_outcomes=("policy-blocked", "escalated"),
            quality_fields=("route_valid", "policy_blocked"),
        ),
        GovernedFixture(
            fixture_id="cancellation-v1",
            kind="cancellation",
            prompt="Cancel a queued or running turn without producing accepted work.",
            expected_outcomes=("cancelled",),
            quality_fields=("route_valid", "cancelled"),
        ),
        GovernedFixture(
            fixture_id="long-context-v1",
            kind="long-context",
            prompt="Use the required long context while preserving bounded review quality.",
            expected_outcomes=("accepted",),
            quality_fields=("route_valid", "plan_valid", "evidence_valid", "accepted"),
            required_context_tokens=7_200,
        ),
        GovernedFixture(
            fixture_id="model-swap-v1",
            kind="model-swap",
            prompt="Detect and report a model residency swap without confusing its digest.",
            expected_outcomes=("accepted",),
            quality_fields=("route_valid", "evidence_valid", "accepted"),
        ),
        GovernedFixture(
            fixture_id="queued-local-agent-v1",
            kind="queued-local-agent",
            prompt="Complete one queued Local Agent turn with explicit Mission attribution.",
            expected_outcomes=("accepted",),
            quality_fields=("route_valid", "plan_valid", "evidence_valid", "accepted"),
        ),
    )


@dataclass(frozen=True)
class RuntimePin:
    """The runtime and binary/configuration identity a promotion may use."""

    runtime_id: str
    runtime_version: str
    binary_digest: str
    configuration_digest: str
    withdrawn: bool = False

    def __post_init__(self) -> None:
        if (
            not isinstance(self.runtime_id, str)
            or not isinstance(self.runtime_version, str)
            or not self.runtime_id.strip()
            or not self.runtime_version.strip()
        ):
            raise ValueError("promotion runtime identity must be non-empty")
        for name, value in (
            ("binary_digest", self.binary_digest),
            ("configuration_digest", self.configuration_digest),
        ):
            if (
                not isinstance(value, str)
                or len(value) != _DIGEST_LENGTH
                or value.lower() != value
                or any(character not in "0123456789abcdef" for character in value)
            ):
                raise ValueError(f"promotion {name} must be a lowercase SHA-256")
        if not isinstance(self.withdrawn, bool):
            raise ValueError("promotion runtime withdrawal state must be boolean")

    def to_dict(self) -> dict[str, Any]:
        return {
            "runtime_id": self.runtime_id,
            "runtime_version": self.runtime_version,
            "binary_digest": self.binary_digest,
            "configuration_digest": self.configuration_digest,
            "withdrawn": self.withdrawn,
        }


@dataclass(frozen=True)
class ContextSource:
    """A bounded source whose digest participates in context selection."""

    source_id: str
    content: str
    required: bool = False

    def __post_init__(self) -> None:
        if not isinstance(self.source_id, str) or not self.source_id.strip():
            raise ValueError("context source id must be non-empty")
        if not isinstance(self.content, str):
            raise ValueError("context source content must be text")
        if not isinstance(self.required, bool):
            raise ValueError("context source required flag must be boolean")

    @property
    def digest(self) -> str:
        return hashlib.sha256(self.content.encode("utf-8")).hexdigest()

    @property
    def token_count(self) -> int:
        return estimate_prompt_tokens(self.content)


@dataclass(frozen=True)
class ContextSelection:
    """The deterministic context result, without any model outcome."""

    cache_key: str
    source_digest: str
    source_ids: tuple[str, ...]
    prompt_prefix: str
    token_count: int
    cache_hit: bool = False


class DeterministicContextSelector:
    """Select context by source digest and bounded budget only."""

    def __init__(self) -> None:
        self._cache: dict[str, ContextSelection] = {}

    def select(
        self,
        sources: tuple[ContextSource, ...] | list[ContextSource],
        *,
        profile_role: str,
        budget_tokens: int,
        required_source_ids: tuple[str, ...] = (),
    ) -> ContextSelection:
        if not isinstance(profile_role, str) or not profile_role.strip():
            raise ValueError("context profile role must be non-empty")
        if (
            not isinstance(budget_tokens, int)
            or isinstance(budget_tokens, bool)
            or budget_tokens <= 0
        ):
            raise ValueError("context selection budget must be positive")
        source_list = tuple(sources)
        if len({source.source_id for source in source_list}) != len(source_list):
            raise ValueError("context source ids must be unique")
        by_id = {source.source_id: source for source in source_list}
        required_ids = tuple(dict.fromkeys(required_source_ids))
        missing = [source_id for source_id in required_ids if source_id not in by_id]
        if missing:
            raise ValueError(f"required context source is missing: {missing[0]}")
        source_identity = [
            {
                "source_id": source.source_id,
                "digest": source.digest,
                "tokens": source.token_count,
                "required": source.required,
            }
            for source in sorted(source_list, key=lambda item: item.source_id)
        ]
        cache_key = hashlib.sha256(
            json.dumps(
                {
                    "profile_role": profile_role,
                    "budget_tokens": budget_tokens,
                    "required_source_ids": required_ids,
                    "sources": source_identity,
                },
                sort_keys=True,
                separators=(",", ":"),
            ).encode("utf-8")
        ).hexdigest()
        cached = self._cache.get(cache_key)
        if cached is not None:
            return replace(cached, cache_hit=True)

        required = [by_id[source_id] for source_id in required_ids]
        required.extend(
            source
            for source in sorted(
                source_list, key=lambda item: (item.digest, item.source_id)
            )
            if source.required and source.source_id not in required_ids
        )
        selected: list[ContextSource] = []
        selected_ids: set[str] = set()
        token_count = 0
        for source in required:
            if source.source_id in selected_ids:
                continue
            next_count = token_count + source.token_count
            if next_count > budget_tokens:
                raise ValueError(
                    f"required context does not fit the {budget_tokens}-token budget"
                )
            selected.append(source)
            selected_ids.add(source.source_id)
            token_count = next_count
        for source in sorted(
            source_list, key=lambda item: (item.digest, item.source_id)
        ):
            if source.source_id in selected_ids:
                continue
            next_count = token_count + source.token_count
            if next_count > budget_tokens:
                continue
            selected.append(source)
            selected_ids.add(source.source_id)
            token_count = next_count
        prompt_prefix = "\n\n".join(
            f"[{source.source_id}]\n{source.content}" for source in selected
        )
        selection = ContextSelection(
            cache_key=cache_key,
            source_digest=hashlib.sha256(
                json.dumps(
                    [
                        {"source_id": source.source_id, "digest": source.digest}
                        for source in selected
                    ],
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode("utf-8")
            ).hexdigest(),
            source_ids=tuple(source.source_id for source in selected),
            prompt_prefix=prompt_prefix,
            token_count=token_count,
        )
        self._cache[cache_key] = selection
        return selection


@dataclass(frozen=True)
class PrefixReuseObservation:
    prefix_digest: str
    reused: bool
    invalidated: bool


class PromptPrefixReuseTracker:
    """Measure exact prefix reuse without retaining model results."""

    def __init__(self) -> None:
        self._last_prefix_digest: str | None = None

    def observe(self, prefix: str) -> PrefixReuseObservation:
        if not isinstance(prefix, str):
            raise TypeError("prompt prefix must be text")
        digest = hashlib.sha256(prefix.encode("utf-8")).hexdigest()
        previous = self._last_prefix_digest
        self._last_prefix_digest = digest
        return PrefixReuseObservation(
            prefix_digest=digest,
            reused=previous == digest,
            invalidated=previous is not None and previous != digest,
        )


@dataclass(frozen=True)
class ContextProfileDecision:
    profile: LocalInferenceProfile
    expanded: bool
    reason: str


class ContextProfilePlanner:
    """Keep the initial bounded context unless required material earns growth."""

    _MAX_BOUNDED_CONTEXT_TOKENS = 65_536

    @classmethod
    def choose(
        cls,
        initial: LocalInferenceProfile,
        expanded: LocalInferenceProfile,
        *,
        required_context_tokens: int,
        initial_quality: float,
        expanded_quality: float,
        initial_reliability: float,
        expanded_reliability: float,
    ) -> ContextProfileDecision:
        if initial.context_budget >= expanded.context_budget:
            raise ValueError(
                "expanded context profile must have a larger bounded budget"
            )
        if expanded.context_budget > cls._MAX_BOUNDED_CONTEXT_TOKENS:
            raise ValueError(
                "expanded context profile exceeds the bounded qualification size"
            )
        if (
            not isinstance(required_context_tokens, int)
            or isinstance(required_context_tokens, bool)
            or required_context_tokens < 0
        ):
            raise ValueError("required context tokens must be non-negative")
        for name, value in (
            ("initial_quality", initial_quality),
            ("expanded_quality", expanded_quality),
            ("initial_reliability", initial_reliability),
            ("expanded_reliability", expanded_reliability),
        ):
            if (
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or not math.isfinite(float(value))
                or not 0 <= value <= 1
            ):
                raise ValueError(f"{name} must be between zero and one")
        initial_material_limit = initial.context_budget - initial.output_budget
        expanded_material_limit = expanded.context_budget - expanded.output_budget
        if required_context_tokens <= initial_material_limit:
            return ContextProfileDecision(
                profile=initial,
                expanded=False,
                reason="required-material-fits-initial-profile",
            )
        if required_context_tokens > expanded_material_limit:
            raise ValueError("required context exceeds the bounded expanded profile")
        improved = (
            expanded_quality > initial_quality
            or expanded_reliability > initial_reliability
        )
        if not improved:
            return ContextProfileDecision(
                profile=initial,
                expanded=False,
                reason="expanded-profile-did-not-improve-reviewed-outcomes",
            )
        return ContextProfileDecision(
            profile=expanded,
            expanded=True,
            reason="required-material-fits-and-reviewed-outcomes-improved",
        )


@dataclass(frozen=True)
class ContextProfileComparison:
    """A role-specific comparison of initial and bounded expanded profiles."""

    role: str
    initial_profile: dict[str, Any]
    expanded_profile: dict[str, Any]
    required_context_tokens: int
    initial_quality: float
    expanded_quality: float
    initial_reliability: float
    expanded_reliability: float
    decision: ContextProfileDecision

    def to_dict(self) -> dict[str, Any]:
        return {
            "role": self.role,
            "initial_profile": dict(self.initial_profile),
            "expanded_profile": dict(self.expanded_profile),
            "required_context_tokens": self.required_context_tokens,
            "initial_quality": self.initial_quality,
            "expanded_quality": self.expanded_quality,
            "initial_reliability": self.initial_reliability,
            "expanded_reliability": self.expanded_reliability,
            "expanded": self.decision.expanded,
            "reason": self.decision.reason,
            "selected_profile_id": self.decision.profile.profile_id,
        }


def compare_context_profiles(
    role: str,
    initial: LocalInferenceProfile,
    expanded: LocalInferenceProfile,
    *,
    required_context_tokens: int,
    initial_quality: float,
    expanded_quality: float,
    initial_reliability: float,
    expanded_reliability: float,
) -> ContextProfileComparison:
    if not isinstance(role, str) or not role.strip():
        raise ValueError("context profile comparison role must be non-empty")
    decision = ContextProfilePlanner.choose(
        initial,
        expanded,
        required_context_tokens=required_context_tokens,
        initial_quality=initial_quality,
        expanded_quality=expanded_quality,
        initial_reliability=initial_reliability,
        expanded_reliability=expanded_reliability,
    )
    return ContextProfileComparison(
        role=role,
        initial_profile=initial.to_dict(),
        expanded_profile=expanded.to_dict(),
        required_context_tokens=required_context_tokens,
        initial_quality=float(initial_quality),
        expanded_quality=float(expanded_quality),
        initial_reliability=float(initial_reliability),
        expanded_reliability=float(expanded_reliability),
        decision=decision,
    )


def default_context_profiles(model: str) -> dict[str, dict[str, LocalInferenceProfile]]:
    """Return the bounded v1 controller/normal-worker comparison profiles."""

    if not isinstance(model, str) or not model.strip():
        raise ValueError("context profile model must be non-empty")
    controller = LocalInferenceProfile.default(
        model,
        profile_id="controller-context-8192-v1",
    )
    worker = replace(
        LocalInferenceProfile.default(
            model,
            profile_id="normal-worker-context-16384-v1",
        ),
        context_budget=16_384,
        output_budget=2_048,
        keep_alive="10m",
    )
    return {
        "controller": {
            "initial": controller,
            "expanded": replace(
                controller,
                profile_id="controller-context-16384-v1",
                context_budget=16_384,
            ),
        },
        "normal-worker": {
            "initial": worker,
            "expanded": replace(
                worker,
                profile_id="normal-worker-context-32768-v1",
                context_budget=32_768,
            ),
        },
    }


@dataclass(frozen=True)
class QualificationReport:
    """Bounded evidence for one repeated Profile qualification cohort."""

    report_id: str
    created_at: str
    profile: dict[str, Any]
    runtime_pin: dict[str, Any]
    fixture_ids: tuple[str, ...]
    repetitions: int
    observations: tuple[dict[str, Any], ...]
    metrics: dict[str, Any]
    promotion_blockers: tuple[str, ...]
    rollback_tested: bool
    baseline_report_id: str = ""

    @property
    def promotion_ready(self) -> bool:
        return not self.promotion_blockers

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": QUALIFICATION_SCHEMA_VERSION,
            "report_id": self.report_id,
            "created_at": self.created_at,
            "profile": json.loads(json.dumps(self.profile, ensure_ascii=True)),
            "runtime_pin": json.loads(json.dumps(self.runtime_pin, ensure_ascii=True)),
            "fixture_ids": list(self.fixture_ids),
            "repetitions": self.repetitions,
            "observations": json.loads(
                json.dumps(self.observations, ensure_ascii=True)
            ),
            "metrics": json.loads(json.dumps(self.metrics, ensure_ascii=True)),
            "promotion_ready": self.promotion_ready,
            "promotion_blockers": list(self.promotion_blockers),
            "rollback_tested": self.rollback_tested,
            "baseline_report_id": self.baseline_report_id,
        }

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "QualificationReport":
        if (
            not isinstance(data, Mapping)
            or data.get("schema_version") != QUALIFICATION_SCHEMA_VERSION
        ):
            raise ValueError("qualification report schema is invalid")
        try:
            json.dumps(data, ensure_ascii=True, allow_nan=False)
        except (TypeError, ValueError) as exc:
            raise ValueError("qualification report contains non-finite data") from exc
        report_id = data.get("report_id")
        created_at = data.get("created_at")
        if not isinstance(report_id, str) or not report_id.strip():
            raise ValueError("qualification report id is invalid")
        if not isinstance(created_at, str) or not created_at.strip():
            raise ValueError("qualification report timestamp is invalid")
        try:
            parsed_at = datetime.fromisoformat(
                created_at.removesuffix("Z")
                + ("+00:00" if created_at.endswith("Z") else "")
            )
            if parsed_at.tzinfo is None:
                raise ValueError("timestamp must include timezone")
        except ValueError as exc:
            raise ValueError("qualification report timestamp is invalid") from exc
        raw_profile = data.get("profile")
        if not isinstance(raw_profile, Mapping):
            raise ValueError("qualification report profile is invalid")
        try:
            profile = LocalInferenceProfile.from_dict(dict(raw_profile))
        except (TypeError, ValueError, RecursionError) as exc:
            raise ValueError("qualification report profile is invalid") from exc
        if dict(raw_profile) != profile.to_dict():
            raise ValueError("qualification report profile is not canonical")
        raw_runtime = data.get("runtime_pin")
        if not isinstance(raw_runtime, Mapping):
            raise ValueError("qualification report runtime pin is invalid")
        try:
            runtime_pin = RuntimePin(
                runtime_id=str(raw_runtime.get("runtime_id", "")),
                runtime_version=str(raw_runtime.get("runtime_version", "")),
                binary_digest=str(raw_runtime.get("binary_digest", "")),
                configuration_digest=str(raw_runtime.get("configuration_digest", "")),
                withdrawn=raw_runtime.get("withdrawn", False),
            )
        except (TypeError, ValueError) as exc:
            raise ValueError("qualification report runtime pin is invalid") from exc
        if dict(raw_runtime) != runtime_pin.to_dict():
            raise ValueError("qualification report runtime pin is not canonical")
        raw_fixture_ids = data.get("fixture_ids")
        raw_observations = data.get("observations")
        raw_blockers = data.get("promotion_blockers")
        if (
            not isinstance(raw_fixture_ids, list)
            or not raw_fixture_ids
            or not all(
                isinstance(item, str) and item.strip() for item in raw_fixture_ids
            )
            or len(raw_fixture_ids) != len(set(raw_fixture_ids))
            or not isinstance(raw_observations, list)
            or not all(isinstance(item, Mapping) for item in raw_observations)
            or not isinstance(raw_blockers, list)
            or not all(isinstance(item, str) and item.strip() for item in raw_blockers)
        ):
            raise ValueError("qualification report collection fields are invalid")
        forbidden_observation_fields = {
            "prompt",
            "raw_output",
            "plan",
            "evidence",
            "authority_decision",
        }
        if any(
            forbidden_observation_fields.intersection(item.keys())
            for item in raw_observations
        ):
            raise ValueError("qualification report contains source-dependent truth")

        def contains_forbidden_truth(value: Any) -> bool:
            if isinstance(value, Mapping):
                if forbidden_observation_fields.intersection(value.keys()):
                    return True
                return any(contains_forbidden_truth(item) for item in value.values())
            if isinstance(value, list):
                return any(contains_forbidden_truth(item) for item in value)
            return False

        if contains_forbidden_truth(data):
            raise ValueError("qualification report contains source-dependent truth")
        repetitions = data.get("repetitions")
        rollback_tested = data.get("rollback_tested")
        baseline_report_id = data.get("baseline_report_id", "")
        raw_metrics = data.get("metrics")
        if (
            not isinstance(repetitions, int)
            or isinstance(repetitions, bool)
            or not 1 <= repetitions <= 32
            or not isinstance(raw_metrics, Mapping)
            or not isinstance(rollback_tested, bool)
            or not isinstance(baseline_report_id, str)
            or data.get("promotion_ready") != (not raw_blockers)
        ):
            raise ValueError("qualification report scalar fields are invalid")
        observations = tuple(dict(item) for item in raw_observations)
        try:
            InferenceQualificationService._validate_persisted_observations(
                observations,
                fixture_ids=tuple(raw_fixture_ids),
                repetitions=repetitions,
                profile_id=profile.profile_id,
            )
            expected_metrics = InferenceQualificationService._metrics(
                list(observations)
            )
            expected_blockers = InferenceQualificationService._promotion_blockers(
                profile=profile,
                observations=list(observations),
                metrics=expected_metrics,
                runtime_pin=runtime_pin,
                rollback_tested=rollback_tested,
                baseline_report=None,
            )
        except (KeyError, TypeError, ValueError, RecursionError) as exc:
            raise ValueError("qualification report observations are invalid") from exc
        if _canonical_json(dict(raw_metrics)) != _canonical_json(expected_metrics):
            raise ValueError("qualification report metrics do not match observations")
        allowed_blockers = expected_blockers
        if baseline_report_id:
            allowed_blockers = [
                *expected_blockers,
                *(
                    blocker
                    for blocker in ("quality-regression", "reliability-regression")
                    if blocker in raw_blockers
                ),
            ]
        if raw_blockers != allowed_blockers:
            raise ValueError("qualification report promotion blockers are invalid")
        return cls(
            report_id=report_id,
            created_at=created_at,
            profile=profile.to_dict(),
            runtime_pin=runtime_pin.to_dict(),
            fixture_ids=tuple(raw_fixture_ids),
            repetitions=repetitions,
            observations=observations,
            metrics=dict(raw_metrics),
            promotion_blockers=tuple(raw_blockers),
            rollback_tested=rollback_tested,
            baseline_report_id=baseline_report_id,
        )


class PromotionError(RuntimeError):
    """Raised when a Local Inference Profile promotion cannot be proven safe."""


class QualificationReportStore:
    """Persist bounded qualification reports and one reversible promotion state."""

    _MAX_REPORT_BYTES = 2 * 1024 * 1024
    _MAX_STATE_BYTES = 256 * 1024

    def __init__(self, runtime_root: Path):
        self.runtime_root = Path(runtime_root).resolve()
        self.root = self.runtime_root / "inference" / "qualification"
        self.reports_root = self.root / "reports"
        self.state_path = self.root / "promotion-state.json"
        self.lock_path = self.root / "promotion-state.lock"

    def save_report(self, report: QualificationReport) -> QualificationReport:
        if not isinstance(report, QualificationReport):
            raise TypeError("qualification report must be a QualificationReport")
        payload = report.to_dict()
        try:
            QualificationReport.from_dict(payload)
        except (TypeError, ValueError, RecursionError) as exc:
            raise ValueError("qualification report is not canonical") from exc
        encoded = (
            json.dumps(payload, ensure_ascii=True, sort_keys=True, indent=2) + "\n"
        )
        if len(encoded.encode("utf-8")) > self._MAX_REPORT_BYTES:
            raise ValueError("qualification report exceeds the bounded size")
        with self._state_lock():
            self.reports_root.mkdir(parents=True, exist_ok=True)
            path = self._report_path(report.report_id)
            if path.exists():
                existing = self.load_report(report.report_id)
                if existing.to_dict() != payload:
                    raise PromotionError(
                        "qualification report id was already used for different evidence"
                    )
                return existing
            temporary = path.with_name(f".{path.name}.{secrets.token_hex(6)}.tmp")
            temporary.write_text(encoded, encoding="utf-8")
            os.replace(temporary, path)
        return report

    def load_report(self, report_id: str) -> QualificationReport:
        return self._load_report(report_id, seen=set())

    def _load_report(self, report_id: str, *, seen: set[str]) -> QualificationReport:
        if report_id in seen:
            raise PromotionError("qualification report baseline cycle is invalid")
        path = self._report_path(report_id)
        try:
            if path.stat().st_size > self._MAX_REPORT_BYTES:
                raise ValueError("qualification report exceeds the bounded size")
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise PromotionError(
                "qualification report is unavailable or invalid"
            ) from exc
        try:
            report = QualificationReport.from_dict(payload)
        except (TypeError, ValueError, RecursionError) as exc:
            raise PromotionError(
                "qualification report is unavailable or invalid"
            ) from exc
        if report.report_id != report_id:
            raise PromotionError(
                "qualification report identity does not match its path"
            )
        baseline = None
        if report.baseline_report_id:
            baseline = self._load_report(
                report.baseline_report_id,
                seen={*seen, report_id},
            )
        expected_blockers = InferenceQualificationService._promotion_blockers(
            profile=LocalInferenceProfile.from_dict(report.profile),
            observations=[dict(item) for item in report.observations],
            metrics=report.metrics,
            runtime_pin=RuntimePin(
                runtime_id=report.runtime_pin["runtime_id"],
                runtime_version=report.runtime_pin["runtime_version"],
                binary_digest=report.runtime_pin["binary_digest"],
                configuration_digest=report.runtime_pin["configuration_digest"],
                withdrawn=report.runtime_pin["withdrawn"],
            ),
            rollback_tested=report.rollback_tested,
            baseline_report=baseline,
        )
        if tuple(expected_blockers) != report.promotion_blockers:
            raise PromotionError("qualification report promotion blockers are invalid")
        return report

    def inspect(self) -> dict[str, Any]:
        if not self.state_path.exists():
            return self._empty_state()
        try:
            if self.state_path.stat().st_size > self._MAX_STATE_BYTES:
                raise ValueError("promotion state exceeds the bounded size")
            payload = json.loads(self.state_path.read_text(encoding="utf-8"))
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise PromotionError("promotion state is invalid") from exc
        return self._validated_state(payload)

    def inspect_reports(self) -> dict[str, Any]:
        """Return bounded report summaries plus promotion state for inspection."""

        reports: list[dict[str, Any]] = []
        if self.reports_root.exists():
            paths = sorted(self.reports_root.glob("*.json"))[:128]
            for path in paths:
                report = self.load_report(self._report_id_from_path(path))
                reports.append(
                    {
                        "report_id": report.report_id,
                        "profile_id": report.profile.get("profile_id", ""),
                        "profile": dict(report.profile),
                        "runtime_pin": dict(report.runtime_pin),
                        "fixture_ids": list(report.fixture_ids),
                        "repetitions": report.repetitions,
                        "metrics": dict(report.metrics),
                        "promotion_ready": report.promotion_ready,
                        "promotion_blockers": list(report.promotion_blockers),
                        "rollback_tested": report.rollback_tested,
                        "baseline_report_id": report.baseline_report_id,
                        "created_at": report.created_at,
                    }
                )
        return {
            "schema_version": QUALIFICATION_SCHEMA_VERSION,
            "reports": reports,
            "promotion": self.inspect(),
        }

    def promote(
        self,
        *,
        profile_id: str,
        report_id: str,
        runtime_pin: RuntimePin,
    ) -> dict[str, Any]:
        report = self.load_report(report_id)
        if report.profile.get("profile_id") != profile_id:
            raise PromotionError(
                "qualification report Profile does not match promotion target"
            )
        if not report.promotion_ready:
            raise PromotionError(
                "Local Inference Profile is not promotion-ready: "
                + ", ".join(report.promotion_blockers)
            )
        if runtime_pin.withdrawn:
            raise PromotionError("withdrawn runtime cannot be promoted")
        if report.runtime_pin != runtime_pin.to_dict():
            raise PromotionError(
                "promotion runtime pin does not match qualification evidence"
            )
        with self._state_lock():
            state = self.inspect()
            active = state.get("active")
            if active is not None and active.get("profile_id") == profile_id:
                if (
                    active.get("report_id") == report_id
                    and active.get("runtime_pin") == runtime_pin.to_dict()
                ):
                    state["last_action"] = "promote-replay"
                    self._write_state(state)
                    return state
                state["previous"] = dict(active)
            elif active is not None:
                state["previous"] = dict(active)
            state["active"] = {
                "profile_id": profile_id,
                "report_id": report_id,
                "profile": dict(report.profile),
                "runtime_pin": runtime_pin.to_dict(),
                "promoted_at": datetime.now(timezone.utc).isoformat(),
            }
            state["last_action"] = "promote"
            state["history"] = [
                *state.get("history", []),
                {"action": "promote", "profile_id": profile_id, "report_id": report_id},
            ][-32:]
            self._write_state(state)
            return state

    def rollback(self, profile_id: str) -> dict[str, Any]:
        with self._state_lock():
            state = self.inspect()
            active = state.get("active")
            if active is None or active.get("profile_id") != profile_id:
                raise PromotionError("profile is not currently promoted")
            previous = state.get("previous")
            state["active"] = dict(previous) if isinstance(previous, dict) else None
            state["previous"] = None
            state["last_action"] = "rollback"
            state["history"] = [
                *state.get("history", []),
                {
                    "action": "rollback",
                    "profile_id": profile_id,
                    "report_id": active.get("report_id", ""),
                },
            ][-32:]
            self._write_state(state)
            return state

    def _report_path(self, report_id: str) -> Path:
        if not isinstance(report_id, str) or not report_id.strip():
            raise PromotionError("qualification report id is invalid")
        return (
            self.reports_root
            / f"{hashlib.sha256(report_id.encode('utf-8')).hexdigest()}.json"
        )

    def _report_id_from_path(self, path: Path) -> str:
        try:
            if path.stat().st_size > self._MAX_REPORT_BYTES:
                raise ValueError("qualification report exceeds the bounded size")
            payload = json.loads(path.read_text(encoding="utf-8"))
            report_id = payload.get("report_id")
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise PromotionError(
                "qualification report is unavailable or invalid"
            ) from exc
        if not isinstance(report_id, str) or self._report_path(report_id) != path:
            raise PromotionError(
                "qualification report identity does not match its path"
            )
        return report_id

    @staticmethod
    def _empty_state() -> dict[str, Any]:
        return {
            "schema_version": QUALIFICATION_SCHEMA_VERSION,
            "active": None,
            "previous": None,
            "history": [],
            "last_action": "none",
        }

    @classmethod
    def _validated_state(cls, payload: Any) -> dict[str, Any]:
        if (
            not isinstance(payload, dict)
            or payload.get("schema_version") != QUALIFICATION_SCHEMA_VERSION
        ):
            raise PromotionError("promotion state is invalid")
        for field_name in ("active", "previous"):
            value = payload.get(field_name)
            if value is not None:
                cls._validated_promotion_record(value)
        history = payload.get("history")
        if (
            not isinstance(history, list)
            or len(history) > 32
            or not all(isinstance(item, dict) for item in history)
            or any(
                not isinstance(item.get("action"), str)
                or not isinstance(item.get("profile_id"), str)
                or not isinstance(item.get("report_id"), str)
                for item in history
            )
        ):
            raise PromotionError("promotion state is invalid")
        if not isinstance(payload.get("last_action"), str):
            raise PromotionError("promotion state is invalid")
        try:
            encoded = json.dumps(payload, ensure_ascii=True, allow_nan=False)
        except (TypeError, ValueError) as exc:
            raise PromotionError("promotion state is invalid") from exc
        if len(encoded.encode("utf-8")) > cls._MAX_STATE_BYTES:
            raise PromotionError("promotion state exceeds the bounded size")
        return json.loads(encoded)

    @staticmethod
    def _validated_promotion_record(value: Any) -> None:
        if not isinstance(value, dict):
            raise PromotionError("promotion state is invalid")
        if any(
            not isinstance(value.get(field_name), str) or not value[field_name].strip()
            for field_name in ("profile_id", "report_id", "promoted_at")
        ):
            raise PromotionError("promotion state is invalid")
        profile = value.get("profile")
        runtime_pin = value.get("runtime_pin")
        if not isinstance(profile, dict) or not isinstance(runtime_pin, dict):
            raise PromotionError("promotion state is invalid")
        try:
            canonical_profile = LocalInferenceProfile.from_dict(profile).to_dict()
            canonical_runtime = RuntimePin(
                runtime_id=str(runtime_pin.get("runtime_id", "")),
                runtime_version=str(runtime_pin.get("runtime_version", "")),
                binary_digest=str(runtime_pin.get("binary_digest", "")),
                configuration_digest=str(runtime_pin.get("configuration_digest", "")),
                withdrawn=runtime_pin.get("withdrawn", False),
            ).to_dict()
        except (TypeError, ValueError, RecursionError) as exc:
            raise PromotionError("promotion state is invalid") from exc
        if profile != canonical_profile or runtime_pin != canonical_runtime:
            raise PromotionError("promotion state is not canonical")

    def _write_state(self, state: dict[str, Any]) -> None:
        validated = self._validated_state(state)
        self.root.mkdir(parents=True, exist_ok=True)
        temporary = self.state_path.with_name(
            f".{self.state_path.name}.{secrets.token_hex(6)}.tmp"
        )
        temporary.write_text(
            json.dumps(validated, ensure_ascii=True, sort_keys=True, indent=2) + "\n",
            encoding="utf-8",
        )
        os.replace(temporary, self.state_path)

    @contextmanager
    def _state_lock(self):
        self.root.mkdir(parents=True, exist_ok=True)
        with self.lock_path.open("a+", encoding="utf-8") as lock_file:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)


class InferenceQualificationService:
    """Run repeated governed fixtures and produce promotion evidence.

    ``runner`` is an integration seam: it receives a fixture, the exact
    candidate Profile, a bounded context projection, and the repetition
    number. Its result is metadata only. The service never caches that result
    or any source-dependent plan, evidence, authority decision, or outcome.
    """

    def __init__(
        self,
        runtime_root: Path,
        *,
        fixtures: tuple[GovernedFixture, ...] | None = None,
        repetitions: int = 3,
        context_selector: DeterministicContextSelector | None = None,
        prefix_tracker: PromptPrefixReuseTracker | None = None,
    ):
        if not isinstance(repetitions, int) or isinstance(repetitions, bool):
            raise ValueError("qualification repetitions must be an integer")
        if not 1 <= repetitions <= 32:
            raise ValueError("qualification repetitions must be between 1 and 32")
        selected = (
            build_governed_fixture_family() if fixtures is None else tuple(fixtures)
        )
        if not selected:
            raise ValueError("qualification requires at least one governed fixture")
        if not all(isinstance(fixture, GovernedFixture) for fixture in selected):
            raise TypeError("qualification fixtures must be GovernedFixture values")
        fixture_ids = [fixture.fixture_id for fixture in selected]
        if len(fixture_ids) != len(set(fixture_ids)):
            raise ValueError("qualification fixture ids must be unique")
        self.runtime_root = Path(runtime_root).resolve()
        self.fixtures = tuple(selected)
        self.repetitions = repetitions
        self.context_selector = context_selector or DeterministicContextSelector()
        self.prefix_tracker = prefix_tracker or PromptPrefixReuseTracker()
        self.report_store = QualificationReportStore(self.runtime_root)

    def qualify(
        self,
        profile: LocalInferenceProfile,
        runner: Callable[
            [GovernedFixture, LocalInferenceProfile, Mapping[str, Any], int],
            Mapping[str, Any],
        ],
        *,
        runtime_pin: RuntimePin,
        rollback_tested: bool = False,
        baseline_report: QualificationReport | None = None,
        report_id: str | None = None,
        context_sources: tuple[ContextSource, ...] | list[ContextSource] = (),
        required_source_ids: tuple[str, ...] = (),
        profile_role: str = "worker",
    ) -> QualificationReport:
        if not isinstance(profile, LocalInferenceProfile):
            raise TypeError("qualification profile must be a LocalInferenceProfile")
        if not isinstance(runtime_pin, RuntimePin):
            raise TypeError("qualification runtime_pin must be a RuntimePin")
        if not isinstance(rollback_tested, bool):
            raise TypeError("qualification rollback_tested must be boolean")
        if not callable(runner):
            raise TypeError("qualification runner must be callable")

        observations: list[dict[str, Any]] = []
        observed_digests: set[str] = set()
        for fixture in self.fixtures:
            for repetition in range(self.repetitions):
                context: dict[str, Any] = {}
                if context_sources:
                    selection = self.context_selector.select(
                        context_sources,
                        profile_role=profile_role,
                        budget_tokens=profile.context_budget - profile.output_budget,
                        required_source_ids=required_source_ids,
                    )
                    prefix = self.prefix_tracker.observe(selection.prompt_prefix)
                    context = {
                        "source_digest": selection.source_digest,
                        "source_ids": selection.source_ids,
                        "token_count": selection.token_count,
                        "cache_hit": selection.cache_hit,
                        "prefix_reused": prefix.reused,
                        "prefix_invalidated": prefix.invalidated,
                    }
                available_context_tokens = (
                    profile.context_budget - profile.output_budget
                )
                if fixture.required_context_tokens > available_context_tokens:
                    result = {
                        "outcome": "rejected",
                        "context_over_budget": True,
                        "unexpected_failure": True,
                        "error": (
                            "Required fixture context exceeds Profile output-headroom budget."
                        ),
                        "timings": {field_name: 0.0 for field_name in _TIMING_FIELDS},
                    }
                else:
                    try:
                        raw_result = runner(fixture, profile, context, repetition)
                        result = self._validated_result(raw_result)
                    except (
                        Exception
                    ) as exc:  # a failed cohort is evidence, not authority
                        result = {
                            "outcome": "escalated",
                            "unexpected_failure": True,
                            "error": str(exc)[:512],
                            "timings": {
                                field_name: 0.0 for field_name in _TIMING_FIELDS
                            },
                        }
                observation = self._observation(
                    fixture,
                    profile,
                    result,
                    repetition=repetition,
                    context=context,
                )
                model_digest = observation.get("model_digest", "")
                if model_digest:
                    observed_digests.add(model_digest)
                observations.append(observation)

        resolved_profile = profile
        if profile.model_digest in {"", "auto"} and len(observed_digests) == 1:
            resolved_profile = replace(
                profile, model_digest=next(iter(observed_digests))
            )
        metrics = self._metrics(observations)
        blockers = self._promotion_blockers(
            profile=resolved_profile,
            observations=observations,
            metrics=metrics,
            runtime_pin=runtime_pin,
            rollback_tested=rollback_tested,
            baseline_report=baseline_report,
        )
        encoded_identity = json.dumps(
            {
                "profile": resolved_profile.to_dict(),
                "runtime_pin": runtime_pin.to_dict(),
                "fixture_ids": [fixture.fixture_id for fixture in self.fixtures],
                "repetitions": self.repetitions,
                "observations": observations,
                "run_nonce": secrets.token_hex(8),
            },
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        selected_report_id = report_id or (
            f"qualification:{resolved_profile.profile_id}:"
            f"{hashlib.sha256(encoded_identity).hexdigest()[:16]}"
        )
        return QualificationReport(
            report_id=selected_report_id,
            created_at=datetime.now(timezone.utc).isoformat(),
            profile=resolved_profile.to_dict(),
            runtime_pin=runtime_pin.to_dict(),
            fixture_ids=tuple(fixture.fixture_id for fixture in self.fixtures),
            repetitions=self.repetitions,
            observations=tuple(observations),
            metrics=metrics,
            promotion_blockers=tuple(blockers),
            rollback_tested=rollback_tested,
            baseline_report_id=baseline_report.report_id if baseline_report else "",
        )

    def qualify_and_save(
        self,
        profile: LocalInferenceProfile,
        runner: Callable[
            [GovernedFixture, LocalInferenceProfile, Mapping[str, Any], int],
            Mapping[str, Any],
        ],
        **kwargs: Any,
    ) -> QualificationReport:
        """Qualify once and persist only the bounded report metadata."""

        report = self.qualify(profile, runner, **kwargs)
        return self.report_store.save_report(report)

    def promote(
        self,
        report: QualificationReport,
        *,
        runtime_pin: RuntimePin,
    ) -> dict[str, Any]:
        """Promote one persisted report after exact pin checks."""

        self.report_store.save_report(report)
        return self.report_store.promote(
            profile_id=str(report.profile["profile_id"]),
            report_id=report.report_id,
            runtime_pin=runtime_pin,
        )

    def rollback(self, profile_id: str) -> dict[str, Any]:
        """Rollback the exact active Profile through the durable store."""

        return self.report_store.rollback(profile_id)

    @staticmethod
    def _validated_result(raw_result: Mapping[str, Any]) -> dict[str, Any]:
        if not isinstance(raw_result, Mapping):
            raise ValueError("qualification runner result must be an object")
        result = dict(raw_result)
        outcome = result.get("outcome")
        if not isinstance(outcome, str) or outcome not in _VALID_OUTCOMES:
            raise ValueError("qualification runner outcome is invalid")
        timings = result.get("timings", {})
        if not isinstance(timings, Mapping):
            raise ValueError("qualification runner timings must be an object")
        for field_name in _TIMING_FIELDS:
            value = timings.get(field_name)
            if (
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or not math.isfinite(float(value))
                or value < 0
            ):
                raise ValueError(f"qualification timing {field_name} is invalid")
        result["timings"] = {
            field_name: float(timings[field_name]) for field_name in _TIMING_FIELDS
        }
        latency = result.get("reviewed_latency_ms")
        if latency is not None and (
            not isinstance(latency, (int, float))
            or isinstance(latency, bool)
            or not math.isfinite(float(latency))
            or latency < 0
        ):
            raise ValueError("qualification reviewed latency is invalid")
        if latency is not None:
            result["reviewed_latency_ms"] = float(latency)
        digest = result.get("model_digest", "")
        if digest and (not isinstance(digest, str) or not digest.strip()):
            raise ValueError("qualification model digest is invalid")
        if digest:
            result["model_digest"] = digest.strip()
        return result

    @staticmethod
    def _validate_persisted_observations(
        observations: tuple[dict[str, Any], ...],
        *,
        fixture_ids: tuple[str, ...],
        repetitions: int,
        profile_id: str,
    ) -> None:
        expected_fields = {
            "fixture_id",
            "fixture_kind",
            "repetition",
            "profile_id",
            "model_digest",
            "route_valid",
            "plan_valid",
            "evidence_valid",
            "accepted",
            "repaired",
            "escalated",
            "policy_blocked",
            "cancelled",
            "model_swapped",
            "queued",
            "context_over_budget",
            "outcome",
            "expected",
            "reliable",
            "quality_score",
            "timings",
            "reviewed_latency_ms",
            "error",
            "context_source_digest",
            "context_cache_hit",
            "prefix_reused",
            "prefix_invalidated",
        }
        if len(observations) != len(fixture_ids) * repetitions:
            raise ValueError("qualification observation count is invalid")
        counts = {fixture_id: 0 for fixture_id in fixture_ids}
        boolean_fields = {
            "route_valid",
            "plan_valid",
            "evidence_valid",
            "accepted",
            "repaired",
            "escalated",
            "policy_blocked",
            "cancelled",
            "model_swapped",
            "queued",
            "context_over_budget",
            "expected",
            "reliable",
            "context_cache_hit",
            "prefix_reused",
            "prefix_invalidated",
        }
        for observation in observations:
            if set(observation) != expected_fields:
                raise ValueError("qualification observation fields are invalid")
            fixture_id = observation["fixture_id"]
            if fixture_id not in counts:
                raise ValueError("qualification observation fixture is invalid")
            counts[fixture_id] += 1
            if (
                not isinstance(observation["fixture_kind"], str)
                or observation["fixture_kind"] not in GOVERNED_FIXTURE_KINDS
                or not isinstance(observation["profile_id"], str)
                or observation["profile_id"] != profile_id
                or not isinstance(observation["model_digest"], str)
                or not isinstance(observation["outcome"], str)
                or observation["outcome"] not in _VALID_OUTCOMES
                or not isinstance(observation["error"], str)
                or len(observation["error"]) > 512
                or not isinstance(observation["context_source_digest"], str)
                or (
                    observation["context_source_digest"]
                    and (
                        len(observation["context_source_digest"]) != _DIGEST_LENGTH
                        or any(
                            character not in "0123456789abcdef"
                            for character in observation["context_source_digest"]
                        )
                    )
                )
            ):
                raise ValueError("qualification observation scalar is invalid")
            if (
                not isinstance(observation["repetition"], int)
                or isinstance(observation["repetition"], bool)
                or not 0 <= observation["repetition"] < repetitions
            ):
                raise ValueError("qualification observation repetition is invalid")
            if any(
                not isinstance(observation[field_name], bool)
                for field_name in boolean_fields
            ):
                raise ValueError("qualification observation flag is invalid")
            quality_score = observation["quality_score"]
            if (
                not isinstance(quality_score, float)
                or not math.isfinite(quality_score)
                or not 0.0 <= quality_score <= 1.0
            ):
                raise ValueError("qualification observation quality is invalid")
            timings = observation["timings"]
            if set(timings) != set(_TIMING_FIELDS) or any(
                not isinstance(timings[field_name], float)
                or not math.isfinite(timings[field_name])
                or timings[field_name] < 0.0
                for field_name in _TIMING_FIELDS
            ):
                raise ValueError("qualification observation timings are invalid")
            latency = observation["reviewed_latency_ms"]
            if latency is not None and (
                not isinstance(latency, float)
                or not math.isfinite(latency)
                or latency < 0.0
            ):
                raise ValueError("qualification observation latency is invalid")
        if any(count != repetitions for count in counts.values()):
            raise ValueError("qualification fixture repetitions are incomplete")

    @classmethod
    def _observation(
        cls,
        fixture: GovernedFixture,
        profile: LocalInferenceProfile,
        result: Mapping[str, Any],
        *,
        repetition: int,
        context: Mapping[str, Any] | None = None,
    ) -> dict[str, Any]:
        outcome = str(result["outcome"])
        route = result.get("route")
        plan = result.get("plan")
        evidence = result.get("evidence")
        flags = {
            "route_valid": bool(result.get("route_valid"))
            if isinstance(result.get("route_valid"), bool)
            else isinstance(route, Mapping)
            and str(route.get("kind", "")) in {"discussion", "coding-task"},
            "plan_valid": bool(result.get("plan_valid"))
            if isinstance(result.get("plan_valid"), bool)
            else isinstance(plan, Mapping) and bool(plan),
            "evidence_valid": bool(result.get("evidence_valid"))
            if isinstance(result.get("evidence_valid"), bool)
            else isinstance(evidence, Mapping) and bool(evidence),
            "accepted": bool(result.get("accepted"))
            if isinstance(result.get("accepted"), bool)
            else outcome == "accepted",
            "repaired": bool(result.get("repaired"))
            if isinstance(result.get("repaired"), bool)
            else outcome == "repaired",
            "escalated": bool(result.get("escalated"))
            if isinstance(result.get("escalated"), bool)
            else outcome == "escalated",
            "policy_blocked": bool(result.get("policy_blocked"))
            if isinstance(result.get("policy_blocked"), bool)
            else outcome == "policy-blocked",
            "cancelled": bool(result.get("cancelled"))
            if isinstance(result.get("cancelled"), bool)
            else outcome == "cancelled",
            "model_swapped": bool(result.get("model_swapped", False)),
            "queued": bool(result.get("queued", False)),
            "context_over_budget": bool(result.get("context_over_budget", False)),
        }
        quality_values = [flags[field_name] for field_name in fixture.quality_fields]
        expected = outcome in fixture.expected_outcomes
        reliable = (
            bool(result.get("reliable"))
            if isinstance(result.get("reliable"), bool)
            else expected and not bool(result.get("unexpected_failure", False))
        )
        latency = result.get("reviewed_latency_ms")
        if flags["accepted"] or flags["repaired"]:
            reliable = reliable and latency is not None
        return {
            "fixture_id": fixture.fixture_id,
            "fixture_kind": fixture.kind,
            "repetition": repetition,
            "profile_id": profile.profile_id,
            "model_digest": str(result.get("model_digest", profile.model_digest)),
            **flags,
            "outcome": outcome,
            "expected": expected,
            "reliable": reliable,
            "quality_score": round(sum(quality_values) / len(quality_values), 4),
            "timings": dict(result["timings"]),
            "reviewed_latency_ms": latency,
            "error": str(result.get("error", ""))[:512],
            "context_source_digest": str((context or {}).get("source_digest", "")),
            "context_cache_hit": bool((context or {}).get("cache_hit", False)),
            "prefix_reused": bool((context or {}).get("prefix_reused", False)),
            "prefix_invalidated": bool(
                (context or {}).get("prefix_invalidated", False)
            ),
        }

    @staticmethod
    def _nearest_rank(values: list[float], percentile: float) -> float | None:
        if not values:
            return None
        ordered = sorted(values)
        rank = max(1, math.ceil(percentile * len(ordered)))
        return float(ordered[rank - 1])

    @classmethod
    def _summary(cls, values: list[float]) -> dict[str, float | None]:
        return {
            "p50": cls._nearest_rank(values, 0.5),
            "p95": cls._nearest_rank(values, 0.95),
        }

    @classmethod
    def _metrics(cls, observations: list[dict[str, Any]]) -> dict[str, Any]:
        latencies = [
            float(item["reviewed_latency_ms"])
            for item in observations
            if item.get("reviewed_latency_ms") is not None
        ]
        timings = {
            field_name: cls._summary(
                [float(item["timings"][field_name]) for item in observations]
            )
            for field_name in _TIMING_FIELDS
        }
        count = len(observations)
        quality = sum(float(item["quality_score"]) for item in observations)
        reliable = sum(1 for item in observations if item["reliable"])
        return {
            "sample_count": count,
            "valid_routes": sum(1 for item in observations if item["route_valid"]),
            "valid_plans": sum(1 for item in observations if item["plan_valid"]),
            "valid_evidence": sum(1 for item in observations if item["evidence_valid"]),
            "accepted_outcomes": sum(1 for item in observations if item["accepted"]),
            "repairs": sum(1 for item in observations if item["repaired"]),
            "escalations": sum(1 for item in observations if item["escalated"]),
            "policy_blocks": sum(1 for item in observations if item["policy_blocked"]),
            "cancellations": sum(1 for item in observations if item["cancelled"]),
            "model_swaps": sum(1 for item in observations if item["model_swapped"]),
            "queued_local_agents": sum(1 for item in observations if item["queued"]),
            "context_over_budget": sum(
                1 for item in observations if item["context_over_budget"]
            ),
            "reliability_rate": round(reliable / count, 4) if count else 0.0,
            "quality_rate": round(quality / count, 4) if count else 0.0,
            "unexpected_outcomes": sum(
                1 for item in observations if not item["expected"]
            ),
            "reviewed_latency_ms": cls._summary(latencies),
            "timings_ms": timings,
            "context_cache_hits": sum(
                1 for item in observations if item["context_cache_hit"]
            ),
            "prefix_reuses": sum(1 for item in observations if item["prefix_reused"]),
            "prefix_invalidations": sum(
                1 for item in observations if item["prefix_invalidated"]
            ),
            "context_source_digests": sorted(
                {
                    item["context_source_digest"]
                    for item in observations
                    if item["context_source_digest"]
                }
            ),
        }

    @staticmethod
    def _promotion_blockers(
        *,
        profile: LocalInferenceProfile,
        observations: list[dict[str, Any]],
        metrics: Mapping[str, Any],
        runtime_pin: RuntimePin,
        rollback_tested: bool,
        baseline_report: QualificationReport | None,
    ) -> list[str]:
        blockers: list[str] = []
        if profile.model_digest in {"", "auto"}:
            blockers.append("exact-model-digest-required")
        elif any(
            item.get("model_digest") not in {"", profile.model_digest}
            for item in observations
        ):
            blockers.append("model-digest-mismatch")
        if not observations or any(not item["reliable"] for item in observations):
            blockers.append("reliability-not-qualified")
        if metrics["quality_rate"] < 1.0:
            blockers.append("quality-not-qualified")
        if metrics["unexpected_outcomes"]:
            blockers.append("unexpected-outcome")
        if not rollback_tested:
            blockers.append("rollback-not-tested")
        if runtime_pin.withdrawn:
            blockers.append("runtime-withdrawn")
        if baseline_report is not None:
            if metrics["quality_rate"] < baseline_report.metrics["quality_rate"]:
                blockers.append("quality-regression")
            if (
                metrics["reliability_rate"]
                < baseline_report.metrics["reliability_rate"]
            ):
                blockers.append("reliability-regression")
        return blockers
