"""Governed Local Inference Profile qualification and promotion contracts.

This module deliberately keeps qualification evidence separate from Mission
authority. It records bounded observations about reviewed work; it never
stores prompts, model streams, plans, Evidence Packages, or source-dependent
outcomes as reusable truth.
"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from datetime import datetime, timezone
import fcntl
import heapq
import hashlib
import json
import math
import os
from pathlib import Path
import secrets
import time
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
_MIN_QUALIFICATION_REPETITIONS = 2
_MAX_FIXTURES = 256
_MAX_FIXTURE_ID_LENGTH = 128
_MAX_FIXTURE_PROMPT_BYTES = 256 * 1024
_MAX_CONTEXT_SOURCES = 256
_MAX_CONTEXT_SOURCE_BYTES = 1 * 1024 * 1024
_MAX_CONTEXT_INPUT_BYTES = 8 * 1024 * 1024
_MAX_CONTEXT_ROLE_LENGTH = 128
_MAX_REQUIRED_CONTEXT_IDS = 256
_MAX_CONTEXT_CACHE_ENTRIES = 256
_MAX_CONTEXT_CACHE_BYTES = 32 * 1024 * 1024
_MAX_PROMOTION_HISTORY = 32
_MAX_RUNNER_RESULT_FIELDS = 64
_MAX_RUNNER_RESULT_BYTES = 256 * 1024
_MAX_CORRELATION_ID_LENGTH = 128
_VALID_ERROR_CODES = {
    "",
    "qualification-cancelled",
    "qualification-deadline-exceeded",
    "context-over-budget",
    "runner-failed",
    "invalid-runner-result",
}
_ROUTE_KINDS = {
    "discussion": "discussion",
    "routing": "discussion",
    "small-edit": "coding-task",
    "multi-file-edit": "coding-task",
    "repair": "coding-task",
    "malformed-output": "discussion",
    "policy-violation": "discussion",
    "cancellation": "discussion",
    "long-context": "coding-task",
    "model-swap": "discussion",
    "queued-local-agent": "coding-task",
}
_PLAN_KINDS = {
    "small-edit",
    "multi-file-edit",
    "repair",
    "long-context",
    "queued-local-agent",
}
_EVIDENCE_KINDS = {
    "small-edit",
    "multi-file-edit",
    "repair",
    "long-context",
    "model-swap",
    "queued-local-agent",
}
_ROLLBACK_CLAIM_FIELDS = {
    "profile_id",
    "runtime_pin",
    "previous_report_id",
    "restored_report_id",
    "replay_verified",
}
_ROLLBACK_EVIDENCE_FIELDS = {
    *_ROLLBACK_CLAIM_FIELDS,
    "report_id",
    "receipt_id",
}


def _runtime_pin_from_dict(raw_runtime: Mapping[str, Any]) -> "RuntimePin":
    if not isinstance(raw_runtime, Mapping):
        raise ValueError("runtime pin must be an object")
    return RuntimePin(
        runtime_id=raw_runtime.get("runtime_id", ""),
        runtime_version=raw_runtime.get("runtime_version", ""),
        binary_digest=raw_runtime.get("binary_digest", ""),
        configuration_digest=raw_runtime.get("configuration_digest", ""),
        withdrawn=raw_runtime.get("withdrawn", False),
    )


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
        if len(self.fixture_id) > _MAX_FIXTURE_ID_LENGTH:
            raise ValueError("governed fixture id is too long")
        if not isinstance(self.kind, str) or self.kind not in GOVERNED_FIXTURE_KINDS:
            raise ValueError(f"unsupported governed fixture kind: {self.kind}")
        if not isinstance(self.prompt, str) or not self.prompt.strip():
            raise ValueError("governed fixture prompt must be non-empty")
        if len(self.prompt.encode("utf-8")) > _MAX_FIXTURE_PROMPT_BYTES:
            raise ValueError("governed fixture prompt exceeds the bounded size")
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
            "model_swapped",
            "queued",
        }
        if (
            not isinstance(self.quality_fields, tuple)
            or not self.quality_fields
            or len(set(self.quality_fields)) != len(self.quality_fields)
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

    @property
    def definition_digest(self) -> str:
        definition = {
            "fixture_id": self.fixture_id,
            "kind": self.kind,
            "prompt": self.prompt,
            "expected_outcomes": self.expected_outcomes,
            "quality_fields": self.quality_fields,
            "required_context_tokens": self.required_context_tokens,
        }
        return hashlib.sha256(_canonical_json(definition).encode("utf-8")).hexdigest()


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
            quality_fields=("cancelled",),
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
            quality_fields=(
                "route_valid",
                "evidence_valid",
                "model_swapped",
                "accepted",
            ),
        ),
        GovernedFixture(
            fixture_id="queued-local-agent-v1",
            kind="queued-local-agent",
            prompt="Complete one queued Local Agent turn with explicit Mission attribution.",
            expected_outcomes=("accepted",),
            quality_fields=(
                "route_valid",
                "plan_valid",
                "evidence_valid",
                "queued",
                "accepted",
            ),
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


def _rollback_receipt_id(evidence: Mapping[str, Any]) -> str:
    payload = {key: value for key, value in evidence.items() if key != "receipt_id"}
    return (
        "rollback-test:"
        + hashlib.sha256(_canonical_json(payload).encode("utf-8")).hexdigest()[:32]
    )


def _validated_rollback_claim(
    raw: Any,
    *,
    profile: LocalInferenceProfile,
    runtime_pin: RuntimePin,
) -> dict[str, Any]:
    if (
        not isinstance(raw, Mapping)
        or len(raw) > len(_ROLLBACK_CLAIM_FIELDS)
        or set(raw) != _ROLLBACK_CLAIM_FIELDS
    ):
        raise ValueError("rollback test evidence is incomplete")
    claim = dict(raw)
    if claim.get("profile_id") != profile.profile_id:
        raise ValueError("rollback test profile identity is invalid")
    raw_runtime_pin = claim.get("runtime_pin")
    expected_runtime_pin = runtime_pin.to_dict()
    if (
        not isinstance(raw_runtime_pin, Mapping)
        or len(raw_runtime_pin) > len(expected_runtime_pin)
        or dict(raw_runtime_pin) != expected_runtime_pin
    ):
        raise ValueError("rollback test runtime identity is invalid")
    previous_report_id = claim.get("previous_report_id")
    restored_report_id = claim.get("restored_report_id")
    if (
        not isinstance(previous_report_id, str)
        or not previous_report_id.strip()
        or len(previous_report_id) > 128
        or restored_report_id != previous_report_id
        or not isinstance(claim.get("replay_verified"), bool)
        or claim["replay_verified"] is not True
    ):
        raise ValueError("rollback test restoration evidence is invalid")
    return {
        "profile_id": profile.profile_id,
        "runtime_pin": expected_runtime_pin,
        "previous_report_id": previous_report_id,
        "restored_report_id": restored_report_id,
        "replay_verified": True,
    }


def _validated_rollback_evidence(
    raw: Any,
    *,
    profile: LocalInferenceProfile,
    runtime_pin: RuntimePin,
    report_id: str,
) -> dict[str, Any]:
    if (
        not isinstance(raw, Mapping)
        or len(raw) > len(_ROLLBACK_EVIDENCE_FIELDS)
        or set(raw) != _ROLLBACK_EVIDENCE_FIELDS
    ):
        raise ValueError("rollback test receipt is invalid")
    claim = _validated_rollback_claim(
        {key: raw[key] for key in _ROLLBACK_CLAIM_FIELDS},
        profile=profile,
        runtime_pin=runtime_pin,
    )
    if raw.get("report_id") != report_id:
        raise ValueError("rollback test report identity is invalid")
    receipt_id = raw.get("receipt_id")
    evidence = {**claim, "report_id": report_id}
    if receipt_id != _rollback_receipt_id(evidence):
        raise ValueError("rollback test receipt is invalid")
    return {**evidence, "receipt_id": receipt_id}


def _baseline_profile_identity(profile: Mapping[str, Any]) -> dict[str, Any]:
    """Return the profile configuration identity used for baseline comparison."""

    return {key: value for key, value in profile.items() if key != "profile_id"}


@dataclass(frozen=True)
class ContextSource:
    """A bounded source whose digest participates in context selection."""

    source_id: str
    content: str
    required: bool = False

    def __post_init__(self) -> None:
        if (
            not isinstance(self.source_id, str)
            or not self.source_id.strip()
            or len(self.source_id) > 128
        ):
            raise ValueError("context source id must be non-empty")
        if not isinstance(self.content, str):
            raise ValueError("context source content must be text")
        if len(self.content.encode("utf-8")) > _MAX_CONTEXT_SOURCE_BYTES:
            raise ValueError("context source exceeds the bounded size")
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
        self._cache_bytes = 0

    def select(
        self,
        sources: tuple[ContextSource, ...] | list[ContextSource],
        *,
        profile_role: str,
        budget_tokens: int,
        required_source_ids: tuple[str, ...] = (),
    ) -> ContextSelection:
        if (
            not isinstance(profile_role, str)
            or not profile_role.strip()
            or len(profile_role) > _MAX_CONTEXT_ROLE_LENGTH
        ):
            raise ValueError("context profile role must be non-empty")
        if (
            not isinstance(budget_tokens, int)
            or isinstance(budget_tokens, bool)
            or budget_tokens <= 0
        ):
            raise ValueError("context selection budget must be positive")
        if not isinstance(sources, (tuple, list)):
            raise TypeError("context sources must be a tuple or list")
        source_list = tuple(sources)
        if len(source_list) > _MAX_CONTEXT_SOURCES:
            raise ValueError("context source count exceeds the bounded limit")
        if not all(isinstance(source, ContextSource) for source in source_list):
            raise TypeError("context sources must be ContextSource values")
        if (
            sum(len(source.content.encode("utf-8")) for source in source_list)
            > _MAX_CONTEXT_INPUT_BYTES
        ):
            raise ValueError("context source input exceeds the bounded size")
        if len({source.source_id for source in source_list}) != len(source_list):
            raise ValueError("context source ids must be unique")
        by_id = {source.source_id: source for source in source_list}
        if not isinstance(required_source_ids, (tuple, list)):
            raise TypeError("required context source ids must be a tuple or list")
        if len(required_source_ids) > _MAX_REQUIRED_CONTEXT_IDS:
            raise ValueError("required context source count exceeds the bounded limit")
        if any(
            not isinstance(source_id, str)
            or not source_id.strip()
            or len(source_id) > 128
            for source_id in required_source_ids
        ):
            raise ValueError("required context source id is invalid")
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
        selection_bytes = len(selection.prompt_prefix.encode("utf-8"))
        while self._cache and (
            len(self._cache) >= _MAX_CONTEXT_CACHE_ENTRIES
            or self._cache_bytes + selection_bytes > _MAX_CONTEXT_CACHE_BYTES
        ):
            oldest_key = next(iter(self._cache))
            evicted = self._cache.pop(oldest_key)
            self._cache_bytes -= len(evicted.prompt_prefix.encode("utf-8"))
        if selection_bytes <= _MAX_CONTEXT_CACHE_BYTES:
            self._cache[cache_key] = selection
            self._cache_bytes += selection_bytes
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
    fixture_digests: dict[str, str]
    repetitions: int
    observations: tuple[dict[str, Any], ...]
    metrics: dict[str, Any]
    promotion_blockers: tuple[str, ...]
    rollback_tested: bool
    fixture_digests_complete: bool = True
    baseline_report_id: str = ""
    rollback_receipt_id: str = ""
    rollback_evidence: dict[str, Any] = field(default_factory=dict)

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
            "fixture_digests": dict(self.fixture_digests),
            "repetitions": self.repetitions,
            "observations": json.loads(
                json.dumps(self.observations, ensure_ascii=True)
            ),
            "metrics": json.loads(json.dumps(self.metrics, ensure_ascii=True)),
            "promotion_ready": self.promotion_ready,
            "promotion_blockers": list(self.promotion_blockers),
            "rollback_tested": self.rollback_tested,
            "fixture_digests_complete": self.fixture_digests_complete,
            "baseline_report_id": self.baseline_report_id,
            "rollback_receipt_id": self.rollback_receipt_id,
            "rollback_evidence": json.loads(
                json.dumps(self.rollback_evidence, ensure_ascii=True)
            ),
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
        raw_fixture_digests = data.get("fixture_digests")
        legacy_fixture_digests = "fixture_digests" not in data
        raw_observations = data.get("observations")
        raw_blockers = data.get("promotion_blockers")
        if (
            not isinstance(raw_fixture_ids, list)
            or not raw_fixture_ids
            or not all(
                isinstance(item, str) and item.strip() for item in raw_fixture_ids
            )
            or len(raw_fixture_ids) != len(set(raw_fixture_ids))
            or (
                raw_fixture_digests is not None
                and (
                    not isinstance(raw_fixture_digests, Mapping)
                    or set(raw_fixture_digests) != set(raw_fixture_ids)
                    or any(
                        not isinstance(value, str)
                        or len(value) != _DIGEST_LENGTH
                        or any(
                            character not in "0123456789abcdef" for character in value
                        )
                        for value in raw_fixture_digests.values()
                    )
                )
            )
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

        if any(contains_forbidden_truth(item) for item in raw_observations):
            raise ValueError("qualification report contains source-dependent truth")
        canonical_fixture_digests = {
            fixture.fixture_id: fixture.definition_digest
            for fixture in build_governed_fixture_family()
        }
        if raw_fixture_digests is None:
            if "fixture_digests" in data:
                raise ValueError("qualification fixture definition digests are invalid")
            if not set(raw_fixture_ids).issubset(canonical_fixture_digests):
                raise ValueError(
                    "legacy qualification report lacks fixture definition digests"
                )
            raw_fixture_digests = {
                fixture_id: canonical_fixture_digests[fixture_id]
                for fixture_id in raw_fixture_ids
            }
        for fixture_id, digest in raw_fixture_digests.items():
            if (
                fixture_id in canonical_fixture_digests
                and digest != canonical_fixture_digests[fixture_id]
            ):
                raise ValueError("qualification fixture definition digest is invalid")
        repetitions = data.get("repetitions")
        rollback_tested = data.get("rollback_tested")
        baseline_report_id = data.get("baseline_report_id", "")
        rollback_receipt_id = data.get("rollback_receipt_id", "")
        legacy_rollback_fields = (
            "rollback_receipt_id" not in data or "rollback_evidence" not in data
        )
        legacy_observation_fields = any(
            "runtime_pin" not in item or "prefix_digest" not in item
            for item in raw_observations
        )
        legacy_report_fields = (
            legacy_fixture_digests
            or legacy_rollback_fields
            or legacy_observation_fields
        )
        raw_rollback_evidence = data.get("rollback_evidence", {})
        raw_metrics = data.get("metrics")
        fixture_digests_complete = data.get(
            "fixture_digests_complete", not legacy_fixture_digests
        )
        if (
            not isinstance(repetitions, int)
            or isinstance(repetitions, bool)
            or not 1 <= repetitions <= 32
            or not isinstance(raw_metrics, Mapping)
            or not isinstance(rollback_tested, bool)
            or not isinstance(baseline_report_id, str)
            or not isinstance(rollback_receipt_id, str)
            or not isinstance(raw_rollback_evidence, Mapping)
            or not isinstance(fixture_digests_complete, bool)
            or len(rollback_receipt_id) > 128
            or (
                rollback_receipt_id
                and (
                    not rollback_receipt_id.startswith("rollback-test:")
                    or len(rollback_receipt_id) != len("rollback-test:") + 32
                    or any(
                        character not in "0123456789abcdef"
                        for character in rollback_receipt_id[len("rollback-test:") :]
                    )
                )
            )
            or (not rollback_tested and rollback_receipt_id)
        ):
            raise ValueError("qualification report scalar fields are invalid")
        raw_blockers = list(raw_blockers)
        if legacy_fixture_digests and fixture_digests_complete:
            raise ValueError("legacy qualification fixture digests are incomplete")
        if (
            not fixture_digests_complete
            and "fixture-digests-incomplete" not in raw_blockers
        ):
            raw_blockers.insert(0, "fixture-digests-incomplete")
        if (
            legacy_observation_fields
            and "observation-identity-incomplete" not in raw_blockers
        ):
            raw_blockers.insert(
                1 if "fixture-digests-incomplete" in raw_blockers else 0,
                "observation-identity-incomplete",
            )
        if not raw_rollback_evidence and rollback_receipt_id and legacy_rollback_fields:
            rollback_receipt_id = ""
        if (
            legacy_rollback_fields
            and rollback_tested
            and "rollback-not-tested" not in raw_blockers
        ):
            insertion_index = next(
                (
                    index
                    for index, blocker in enumerate(raw_blockers)
                    if blocker in {"runtime-withdrawn", "baseline-required"}
                ),
                len(raw_blockers),
            )
            raw_blockers.insert(insertion_index, "rollback-not-tested")
        if not isinstance(data.get("promotion_ready"), bool):
            raise ValueError("qualification report scalar fields are invalid")
        if (
            data.get("promotion_ready") != (not raw_blockers)
            and not legacy_report_fields
        ):
            raise ValueError("qualification report scalar fields are invalid")
        try:
            rollback_evidence = (
                _validated_rollback_evidence(
                    raw_rollback_evidence,
                    profile=profile,
                    runtime_pin=runtime_pin,
                    report_id=report_id,
                )
                if raw_rollback_evidence
                else {}
            )
        except (TypeError, ValueError, RecursionError) as exc:
            raise ValueError(
                "qualification report rollback evidence is invalid"
            ) from exc
        if rollback_evidence and (
            not rollback_tested
            or rollback_receipt_id != rollback_evidence["receipt_id"]
        ):
            raise ValueError("qualification report rollback evidence is invalid")
        if not rollback_evidence and rollback_receipt_id:
            raise ValueError("qualification report rollback evidence is invalid")
        observations = tuple(dict(item) for item in raw_observations)
        try:
            InferenceQualificationService._validate_persisted_observations(
                observations,
                fixture_ids=tuple(raw_fixture_ids),
                repetitions=repetitions,
                profile_id=profile.profile_id,
                runtime_pin=runtime_pin,
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
                rollback_receipt_id=rollback_receipt_id,
                rollback_evidence=rollback_evidence,
                baseline_report=None,
                fixture_ids=tuple(raw_fixture_ids),
                fixture_digests=dict(raw_fixture_digests),
                fixture_digests_complete=fixture_digests_complete,
                repetitions=repetitions,
            )
        except (KeyError, TypeError, ValueError, RecursionError) as exc:
            raise ValueError("qualification report observations are invalid") from exc
        if _canonical_json(dict(raw_metrics)) != _canonical_json(expected_metrics):
            raise ValueError("qualification report metrics do not match observations")
        allowed_blockers = expected_blockers
        if baseline_report_id:
            allowed_blockers = [
                blocker
                for blocker in allowed_blockers
                if blocker != "baseline-required"
            ]
            allowed_blockers = [
                *allowed_blockers,
                *(
                    blocker
                    for blocker in (
                        "quality-regression",
                        "reliability-regression",
                        "baseline-incomparable",
                    )
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
            fixture_digests=dict(raw_fixture_digests),
            repetitions=repetitions,
            observations=observations,
            metrics=dict(raw_metrics),
            promotion_blockers=tuple(raw_blockers),
            rollback_tested=rollback_tested,
            fixture_digests_complete=fixture_digests_complete,
            baseline_report_id=baseline_report_id,
            rollback_receipt_id=rollback_receipt_id,
            rollback_evidence=rollback_evidence,
        )


class PromotionError(RuntimeError):
    """Raised when a Local Inference Profile promotion cannot be proven safe."""


class QualificationReportStore:
    """Persist bounded qualification reports and one reversible promotion state."""

    _MAX_REPORT_BYTES = 2 * 1024 * 1024
    _MAX_STATE_BYTES = 256 * 1024
    _MAX_REPORT_COUNT = 128

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
            baseline_report = None
            if report.baseline_report_id:
                baseline_report = self.load_report(report.baseline_report_id)
            self._validate_rollback_target(report, baseline_report=baseline_report)
            self.reports_root.mkdir(parents=True, exist_ok=True)
            path = self._report_path(report.report_id)
            if path.exists():
                existing = self.load_report(report.report_id)
                if existing.to_dict() != payload:
                    raise PromotionError(
                        "qualification report id was already used for different evidence"
                    )
                return existing
            if (
                sum(1 for _ in self.reports_root.glob("*.json"))
                >= self._MAX_REPORT_COUNT
            ):
                raise PromotionError("qualification report capacity exhausted")
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
        self._validate_rollback_target(
            report,
            baseline_report=baseline,
            seen=seen,
        )
        expected_blockers = InferenceQualificationService._promotion_blockers(
            profile=LocalInferenceProfile.from_dict(report.profile),
            observations=[dict(item) for item in report.observations],
            metrics=report.metrics,
            runtime_pin=_runtime_pin_from_dict(report.runtime_pin),
            rollback_tested=report.rollback_tested,
            rollback_receipt_id=report.rollback_receipt_id,
            rollback_evidence=report.rollback_evidence,
            baseline_report=baseline,
            fixture_ids=report.fixture_ids,
            fixture_digests=report.fixture_digests,
            fixture_digests_complete=report.fixture_digests_complete,
            repetitions=report.repetitions,
        )
        if tuple(expected_blockers) != report.promotion_blockers:
            raise PromotionError("qualification report promotion blockers are invalid")
        return report

    def _validate_rollback_target(
        self,
        report: QualificationReport,
        *,
        baseline_report: QualificationReport | None,
        seen: set[str] | frozenset[str] = frozenset(),
    ) -> None:
        """Require every persisted rollback receipt to name a stored report."""

        if not report.rollback_evidence:
            return
        previous_report_id = report.rollback_evidence["previous_report_id"]
        if (
            baseline_report is not None
            and previous_report_id == baseline_report.report_id
        ):
            previous_report = baseline_report
        else:
            try:
                previous_report = self._load_report(
                    previous_report_id,
                    seen={*seen, report.report_id},
                )
            except PromotionError as exc:
                raise PromotionError(
                    "qualification report rollback target is invalid"
                ) from exc
        if previous_report.profile.get("profile_id") != report.profile.get(
            "profile_id"
        ):
            raise PromotionError("qualification report rollback target is invalid")

    def inspect(self) -> dict[str, Any]:
        if not self.state_path.exists():
            return self._empty_state()
        try:
            if self.state_path.stat().st_size > self._MAX_STATE_BYTES:
                raise ValueError("promotion state exceeds the bounded size")
            payload = json.loads(self.state_path.read_text(encoding="utf-8"))
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise PromotionError("promotion state is invalid") from exc
        state = self._validated_state(payload)
        for record_name in ("active", "previous"):
            record = state.get(record_name)
            if record is None:
                continue
            try:
                report = self.load_report(record["report_id"])
            except PromotionError as exc:
                raise PromotionError(
                    "promotion state references an invalid report"
                ) from exc
            if (
                record["profile_id"] != report.profile["profile_id"]
                or record["profile"] != report.profile
                or record["runtime_pin"] != report.runtime_pin
            ):
                raise PromotionError("promotion state report identity is invalid")
        return state

    def inspect_reports(self) -> dict[str, Any]:
        """Return bounded report summaries plus promotion state for inspection."""

        reports: list[dict[str, Any]] = []
        reports_truncated = False
        if self.reports_root.exists():
            paths = heapq.nsmallest(
                self._MAX_REPORT_COUNT + 1,
                self.reports_root.glob("*.json"),
                key=lambda path: path.name,
            )
            reports_truncated = len(paths) > self._MAX_REPORT_COUNT
            paths = paths[: self._MAX_REPORT_COUNT]
            for path in paths:
                report = self.load_report(self._report_id_from_path(path))
                reports.append(
                    {
                        "report_id": report.report_id,
                        "profile_id": report.profile.get("profile_id", ""),
                        "profile": dict(report.profile),
                        "runtime_pin": dict(report.runtime_pin),
                        "fixture_ids": list(report.fixture_ids),
                        "fixture_digests": dict(report.fixture_digests),
                        "fixture_digests_complete": report.fixture_digests_complete,
                        "repetitions": report.repetitions,
                        "metrics": dict(report.metrics),
                        "promotion_ready": report.promotion_ready,
                        "promotion_blockers": list(report.promotion_blockers),
                        "rollback_tested": report.rollback_tested,
                        "rollback_receipt_id": report.rollback_receipt_id,
                        "rollback_evidence": dict(report.rollback_evidence),
                        "baseline_report_id": report.baseline_report_id,
                        "created_at": report.created_at,
                    }
                )
        return {
            "schema_version": QUALIFICATION_SCHEMA_VERSION,
            "reports": reports,
            "reports_truncated": reports_truncated,
            "promotion": self.inspect(),
        }

    def promote(
        self,
        *,
        profile_id: str,
        report_id: str,
        runtime_pin: RuntimePin,
        correlation_id: str,
        expected_revision: int,
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
        request_digest = self._mutation_digest(
            "promote",
            profile_id=profile_id,
            report_id=report_id,
            runtime_pin=runtime_pin,
            correlation_id=correlation_id,
            expected_revision=expected_revision,
        )
        with self._state_lock():
            state = self.inspect()
            replay = self._check_replay_or_revision(
                state,
                correlation_id=correlation_id,
                request_digest=request_digest,
                expected_revision=expected_revision,
            )
            if replay:
                state["last_action"] = "promote-replay"
                return state
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
            if len(state.get("history", [])) >= _MAX_PROMOTION_HISTORY:
                raise PromotionError("promotion history capacity exhausted")
            state["active"] = {
                "profile_id": profile_id,
                "report_id": report_id,
                "profile": dict(report.profile),
                "runtime_pin": runtime_pin.to_dict(),
                "promoted_at": datetime.now(timezone.utc).isoformat(),
            }
            state["last_action"] = "promote"
            state["revision"] += 1
            state["history"] = [
                *state.get("history", []),
                {
                    "action": "promote",
                    "profile_id": profile_id,
                    "report_id": report_id,
                    "correlation_id": correlation_id,
                    "request_digest": request_digest,
                    "revision": state["revision"],
                },
            ]
            self._write_state(state)
            return state

    def rollback(
        self,
        profile_id: str,
        *,
        correlation_id: str,
        expected_revision: int,
    ) -> dict[str, Any]:
        request_digest = self._mutation_digest(
            "rollback",
            profile_id=profile_id,
            report_id="",
            runtime_pin=None,
            correlation_id=correlation_id,
            expected_revision=expected_revision,
        )
        with self._state_lock():
            state = self.inspect()
            replay = self._check_replay_or_revision(
                state,
                correlation_id=correlation_id,
                request_digest=request_digest,
                expected_revision=expected_revision,
            )
            if replay:
                state["last_action"] = "rollback-replay"
                return state
            active = state.get("active")
            if active is None or active.get("profile_id") != profile_id:
                raise PromotionError("profile is not currently promoted")
            previous = state.get("previous")
            if len(state.get("history", [])) >= _MAX_PROMOTION_HISTORY:
                raise PromotionError("promotion history capacity exhausted")
            state["active"] = dict(previous) if isinstance(previous, dict) else None
            state["previous"] = None
            state["last_action"] = "rollback"
            state["revision"] += 1
            state["history"] = [
                *state.get("history", []),
                {
                    "action": "rollback",
                    "profile_id": profile_id,
                    "report_id": active.get("report_id", ""),
                    "correlation_id": correlation_id,
                    "request_digest": request_digest,
                    "revision": state["revision"],
                },
            ]
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
            if not isinstance(payload, Mapping):
                raise ValueError("qualification report root must be an object")
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
            "revision": 0,
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
        if "revision" not in payload:
            payload = {"revision": 0, **payload}
        if (
            not isinstance(payload.get("revision"), int)
            or isinstance(payload.get("revision"), bool)
            or payload["revision"] < 0
        ):
            raise PromotionError("promotion state is invalid")
        for field_name in ("active", "previous"):
            value = payload.get(field_name)
            if value is not None:
                cls._validated_promotion_record(value)
        history = payload.get("history")
        if (
            not isinstance(history, list)
            or len(history) > _MAX_PROMOTION_HISTORY
            or not all(isinstance(item, dict) for item in history)
            or any(
                not isinstance(item.get("action"), str)
                or not isinstance(item.get("profile_id"), str)
                or not isinstance(item.get("report_id"), str)
                or not isinstance(item.get("correlation_id"), str)
                or not item["correlation_id"].strip()
                or len(item["correlation_id"]) > _MAX_CORRELATION_ID_LENGTH
                or not isinstance(item.get("request_digest"), str)
                or len(item["request_digest"]) != _DIGEST_LENGTH
                or any(
                    character not in "0123456789abcdef"
                    for character in item["request_digest"]
                )
                or not isinstance(item.get("revision"), int)
                or isinstance(item["revision"], bool)
                or item["revision"] < 1
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
        if value["profile_id"] != profile.get("profile_id"):
            raise PromotionError("promotion state profile identity is invalid")
        if runtime_pin.get("withdrawn"):
            raise PromotionError("withdrawn runtime cannot be active")

    @staticmethod
    def _mutation_digest(
        action: str,
        *,
        profile_id: str,
        report_id: str,
        runtime_pin: RuntimePin | None,
        correlation_id: str,
        expected_revision: int,
    ) -> str:
        if (
            not isinstance(correlation_id, str)
            or not correlation_id.strip()
            or len(correlation_id) > _MAX_CORRELATION_ID_LENGTH
        ):
            raise PromotionError("qualification mutation correlation id is required")
        if (
            not isinstance(expected_revision, int)
            or isinstance(expected_revision, bool)
            or expected_revision < 0
        ):
            raise PromotionError("qualification mutation expected revision is invalid")
        payload = {
            "action": action,
            "profile_id": profile_id,
            "report_id": report_id,
            "runtime_pin": runtime_pin.to_dict() if runtime_pin else None,
            "correlation_id": correlation_id,
            "expected_revision": expected_revision,
        }
        return hashlib.sha256(_canonical_json(payload).encode("utf-8")).hexdigest()

    @staticmethod
    def _check_replay_or_revision(
        state: Mapping[str, Any],
        *,
        correlation_id: str,
        request_digest: str,
        expected_revision: int,
    ) -> bool:
        for item in state.get("history", []):
            if item.get("correlation_id") == correlation_id:
                if item.get("request_digest") == request_digest:
                    return True
                raise PromotionError("qualification mutation correlation was reused")
        if state.get("revision") != expected_revision:
            raise PromotionError("qualification mutation expected revision is stale")
        return False

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
        if fixtures is not None and not isinstance(fixtures, (tuple, list)):
            raise TypeError("qualification fixtures must be a tuple or list")
        selected = (
            build_governed_fixture_family() if fixtures is None else tuple(fixtures)
        )
        if not selected:
            raise ValueError("qualification requires at least one governed fixture")
        if len(selected) > _MAX_FIXTURES:
            raise ValueError("qualification fixture count exceeds the bounded limit")
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
        rollback_test: Callable[
            [LocalInferenceProfile, RuntimePin, Mapping[str, Any]], Mapping[str, Any]
        ]
        | None = None,
        baseline_report: QualificationReport | None = None,
        report_id: str | None = None,
        context_sources: tuple[ContextSource, ...] | list[ContextSource] = (),
        required_source_ids: tuple[str, ...] = (),
        profile_role: str = "worker",
        cancel_check: Callable[[], bool] | None = None,
        deadline_seconds: float | None = None,
    ) -> QualificationReport:
        if not isinstance(profile, LocalInferenceProfile):
            raise TypeError("qualification profile must be a LocalInferenceProfile")
        if not isinstance(runtime_pin, RuntimePin):
            raise TypeError("qualification runtime_pin must be a RuntimePin")
        if not isinstance(rollback_tested, bool):
            raise TypeError("qualification rollback_tested must be boolean")
        if rollback_test is not None and not callable(rollback_test):
            raise TypeError("qualification rollback_test must be callable")
        if cancel_check is not None and not callable(cancel_check):
            raise TypeError("qualification cancel_check must be callable")
        if deadline_seconds is not None and (
            not isinstance(deadline_seconds, (int, float))
            or isinstance(deadline_seconds, bool)
            or not math.isfinite(float(deadline_seconds))
            or not 0 < deadline_seconds <= 3_600
        ):
            raise ValueError("qualification deadline must be between zero and one hour")
        if not callable(runner):
            raise TypeError("qualification runner must be callable")

        deadline_at = (
            time.monotonic() + float(deadline_seconds)
            if deadline_seconds is not None
            else None
        )
        cancellation_requested = False

        def refresh_control() -> bool:
            nonlocal cancellation_requested
            if not cancellation_requested and cancel_check is not None:
                try:
                    cancellation_requested = bool(cancel_check())
                except Exception:
                    cancellation_requested = True
            if deadline_at is not None and time.monotonic() >= deadline_at:
                cancellation_requested = True
            return cancellation_requested

        def cancellation_result(fixture: GovernedFixture) -> dict[str, Any]:
            return {
                "outcome": "cancelled",
                "unexpected_failure": fixture.kind != "cancellation",
                "error_code": (
                    "qualification-deadline-exceeded"
                    if deadline_at is not None and time.monotonic() >= deadline_at
                    else "qualification-cancelled"
                ),
                "timings": {field_name: None for field_name in _TIMING_FIELDS},
            }

        rollback_claim: dict[str, Any] = {}
        control_context = {
            "cancel_check": cancel_check,
            "deadline_at": deadline_at,
        }
        if rollback_test is not None and not refresh_control():
            try:
                rollback_claim = _validated_rollback_claim(
                    rollback_test(profile, runtime_pin, control_context),
                    profile=profile,
                    runtime_pin=runtime_pin,
                )
                if refresh_control():
                    rollback_claim = {}
            except Exception:
                rollback_claim = {}
        if rollback_claim:
            try:
                previous_report = self.report_store.load_report(
                    rollback_claim["previous_report_id"]
                )
                if previous_report.profile.get("profile_id") != profile.profile_id or (
                    baseline_report is not None
                    and rollback_claim["previous_report_id"]
                    != baseline_report.report_id
                ):
                    rollback_claim = {}
            except PromotionError:
                rollback_claim = {}
        rollback_tested = bool(rollback_claim)
        rollback_receipt_id = ""
        expected_runtime_pin = runtime_pin.to_dict()

        observations: list[dict[str, Any]] = []
        observed_digests: set[str] = set()
        for fixture in self.fixtures:
            for repetition in range(self.repetitions):
                context: dict[str, Any] = {}
                refresh_control()
                available_context_tokens = (
                    profile.context_budget - profile.output_budget
                )
                if cancellation_requested:
                    result = cancellation_result(fixture)
                else:
                    if context_sources:
                        selection = self.context_selector.select(
                            context_sources,
                            profile_role=profile_role,
                            budget_tokens=available_context_tokens,
                            required_source_ids=required_source_ids,
                        )
                        prompt_prefix = f"{selection.prompt_prefix}\n\n{fixture.prompt}"
                        prefix = self.prefix_tracker.observe(selection.prompt_prefix)
                    else:
                        prompt_prefix = fixture.prompt
                        prefix = self.prefix_tracker.observe(fixture.prompt)
                    combined_context_tokens = estimate_prompt_tokens(prompt_prefix)
                    context = {
                        "source_digest": (
                            selection.source_digest if context_sources else ""
                        ),
                        "source_ids": (selection.source_ids if context_sources else ()),
                        "token_count": combined_context_tokens,
                        "context_over_budget": combined_context_tokens
                        > available_context_tokens,
                        "cache_hit": selection.cache_hit if context_sources else False,
                        "prefix_digest": prefix.prefix_digest,
                        "prefix_reused": prefix.reused,
                        "prefix_invalidated": prefix.invalidated,
                        "prompt_prefix": prompt_prefix,
                        "cancel_check": cancel_check,
                        "deadline_at": deadline_at,
                        "runtime_pin": runtime_pin.to_dict(),
                    }
                    refresh_control()
                    if cancellation_requested:
                        result = cancellation_result(fixture)
                    elif (
                        context["context_over_budget"]
                        or fixture.required_context_tokens > available_context_tokens
                    ):
                        result = {
                            "outcome": "rejected",
                            "context_over_budget": True,
                            "unexpected_failure": True,
                            "error_code": "context-over-budget",
                            "timings": {
                                field_name: None for field_name in _TIMING_FIELDS
                            },
                        }
                    else:
                        try:
                            raw_result = runner(fixture, profile, context, repetition)
                            result = self._validated_result(raw_result)
                            if result.get("runtime_pin") != expected_runtime_pin:
                                raise ValueError(
                                    "qualification runner runtime identity does not match"
                                )
                        except Exception:  # a failed cohort is evidence, not authority
                            result = {
                                "outcome": "escalated",
                                "unexpected_failure": True,
                                "error_code": "runner-failed",
                                "timings": {
                                    field_name: None for field_name in _TIMING_FIELDS
                                },
                            }
                        if refresh_control():
                            result = cancellation_result(fixture)
                observation = self._observation(
                    fixture,
                    profile,
                    result,
                    repetition=repetition,
                    runtime_pin=runtime_pin,
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
        rollback_evidence: dict[str, Any] = {}
        if rollback_claim:
            rollback_evidence = {
                **rollback_claim,
                "report_id": selected_report_id,
            }
            rollback_evidence["receipt_id"] = _rollback_receipt_id(rollback_evidence)
            rollback_evidence = _validated_rollback_evidence(
                rollback_evidence,
                profile=resolved_profile,
                runtime_pin=runtime_pin,
                report_id=selected_report_id,
            )
            rollback_receipt_id = rollback_evidence["receipt_id"]
        metrics = self._metrics(observations)
        blockers = self._promotion_blockers(
            profile=resolved_profile,
            observations=observations,
            metrics=metrics,
            runtime_pin=runtime_pin,
            rollback_tested=rollback_tested,
            rollback_receipt_id=rollback_receipt_id,
            rollback_evidence=rollback_evidence,
            baseline_report=baseline_report,
            fixture_ids=tuple(fixture.fixture_id for fixture in self.fixtures),
            fixture_digests={
                fixture.fixture_id: fixture.definition_digest
                for fixture in self.fixtures
            },
            fixture_digests_complete=True,
            repetitions=self.repetitions,
        )
        return QualificationReport(
            report_id=selected_report_id,
            created_at=datetime.now(timezone.utc).isoformat(),
            profile=resolved_profile.to_dict(),
            runtime_pin=runtime_pin.to_dict(),
            fixture_ids=tuple(fixture.fixture_id for fixture in self.fixtures),
            fixture_digests={
                fixture.fixture_id: fixture.definition_digest
                for fixture in self.fixtures
            },
            repetitions=self.repetitions,
            observations=tuple(observations),
            metrics=metrics,
            promotion_blockers=tuple(blockers),
            rollback_tested=rollback_tested,
            baseline_report_id=baseline_report.report_id if baseline_report else "",
            rollback_receipt_id=rollback_receipt_id,
            rollback_evidence=rollback_evidence,
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

        baseline_report = kwargs.get("baseline_report")
        if isinstance(baseline_report, QualificationReport):
            self.report_store.save_report(baseline_report)
        report = self.qualify(profile, runner, **kwargs)
        return self.report_store.save_report(report)

    def promote(
        self,
        report: QualificationReport,
        *,
        runtime_pin: RuntimePin,
        correlation_id: str,
        expected_revision: int,
    ) -> dict[str, Any]:
        """Promote one persisted report after exact pin checks."""

        self.report_store.save_report(report)
        return self.report_store.promote(
            profile_id=str(report.profile["profile_id"]),
            report_id=report.report_id,
            runtime_pin=runtime_pin,
            correlation_id=correlation_id,
            expected_revision=expected_revision,
        )

    def rollback(
        self,
        profile_id: str,
        *,
        correlation_id: str,
        expected_revision: int,
    ) -> dict[str, Any]:
        """Rollback the exact active Profile through the durable store."""

        return self.report_store.rollback(
            profile_id,
            correlation_id=correlation_id,
            expected_revision=expected_revision,
        )

    @staticmethod
    def _validated_result(raw_result: Mapping[str, Any]) -> dict[str, Any]:
        if not isinstance(raw_result, Mapping):
            raise ValueError("qualification runner result must be an object")
        if len(raw_result) > _MAX_RUNNER_RESULT_FIELDS:
            raise ValueError("qualification runner result is too large")
        result = dict(raw_result)
        try:
            encoded_result = json.dumps(
                result,
                ensure_ascii=True,
                allow_nan=False,
                separators=(",", ":"),
            ).encode("utf-8")
        except (TypeError, ValueError, RecursionError) as exc:
            raise ValueError("qualification runner result is not JSON-safe") from exc
        if len(encoded_result) > _MAX_RUNNER_RESULT_BYTES:
            raise ValueError("qualification runner result is too large")
        outcome = result.get("outcome")
        if not isinstance(outcome, str) or outcome not in _VALID_OUTCOMES:
            raise ValueError("qualification runner outcome is invalid")
        timings = result.get("timings", {})
        if not isinstance(timings, Mapping):
            raise ValueError("qualification runner timings must be an object")
        allow_missing_timings = outcome == "cancelled"
        for field_name in _TIMING_FIELDS:
            value = timings.get(field_name)
            if value is None and allow_missing_timings:
                continue
            if (
                not isinstance(value, (int, float))
                or isinstance(value, bool)
                or not math.isfinite(float(value))
                or value < 0
            ):
                raise ValueError(f"qualification timing {field_name} is invalid")
        result["timings"] = {
            field_name: (
                None
                if timings[field_name] is None and allow_missing_timings
                else float(timings[field_name])
            )
            for field_name in _TIMING_FIELDS
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
        if not isinstance(digest, str):
            raise ValueError("qualification model digest is invalid")
        if digest and not digest.strip():
            raise ValueError("qualification model digest is invalid")
        result["model_digest"] = digest.strip()
        error_code = result.get("error_code", "")
        if not isinstance(error_code, str) or error_code not in _VALID_ERROR_CODES:
            raise ValueError("qualification error code is invalid")
        result["error_code"] = error_code
        for field_name in (
            "model_swapped",
            "queued",
            "context_over_budget",
            "unexpected_failure",
        ):
            if field_name in result and not isinstance(result[field_name], bool):
                raise ValueError(f"qualification result {field_name} must be boolean")
        return result

    @staticmethod
    def _validate_persisted_observations(
        observations: tuple[dict[str, Any], ...],
        *,
        fixture_ids: tuple[str, ...],
        repetitions: int,
        profile_id: str,
        runtime_pin: RuntimePin,
    ) -> None:
        has_runtime_bindings = all("runtime_pin" in item for item in observations)
        has_prefix_digests = all("prefix_digest" in item for item in observations)
        if has_runtime_bindings != has_prefix_digests:
            raise ValueError("qualification observation identity fields are incomplete")
        observation_identity_complete = has_runtime_bindings and has_prefix_digests
        expected_fields = {
            "fixture_id",
            "fixture_kind",
            "repetition",
            "profile_id",
            "runtime_pin",
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
            "error_code",
            "context_source_digest",
            "context_cache_hit",
            "prefix_digest",
            "prefix_reused",
            "prefix_invalidated",
        }
        if not observation_identity_complete:
            expected_fields -= {"runtime_pin", "prefix_digest"}
        if len(observations) != len(fixture_ids) * repetitions:
            raise ValueError("qualification observation count is invalid")
        counts = {fixture_id: set() for fixture_id in fixture_ids}
        canonical_fixtures = {
            fixture.fixture_id: fixture for fixture in build_governed_fixture_family()
        }
        canonical_fixture_kinds = {
            fixture_id: fixture.kind
            for fixture_id, fixture in canonical_fixtures.items()
        }
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
        expected_runtime_pin = runtime_pin.to_dict()
        previous_prefix_digest: str | None = None
        for observation in observations:
            if set(observation) != expected_fields:
                raise ValueError("qualification observation fields are invalid")
            fixture_id = observation["fixture_id"]
            if fixture_id not in counts:
                raise ValueError("qualification observation fixture is invalid")
            repetition = observation["repetition"]
            if repetition in counts[fixture_id]:
                raise ValueError("qualification fixture repetition is duplicated")
            counts[fixture_id].add(repetition)
            if (
                not isinstance(observation["fixture_kind"], str)
                or observation["fixture_kind"] not in GOVERNED_FIXTURE_KINDS
                or (
                    fixture_id in canonical_fixture_kinds
                    and observation["fixture_kind"]
                    != canonical_fixture_kinds[fixture_id]
                )
                or not isinstance(observation["profile_id"], str)
                or observation["profile_id"] != profile_id
                or not isinstance(observation["model_digest"], str)
                or not isinstance(observation["outcome"], str)
                or observation["outcome"] not in _VALID_OUTCOMES
                or not isinstance(observation["error_code"], str)
                or observation["error_code"] not in _VALID_ERROR_CODES
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
            if observation_identity_complete and (
                not isinstance(observation["runtime_pin"], Mapping)
                or dict(observation["runtime_pin"]) != expected_runtime_pin
                or not isinstance(observation["prefix_digest"], str)
                or (
                    observation["prefix_digest"]
                    and (
                        len(observation["prefix_digest"]) != _DIGEST_LENGTH
                        or any(
                            character not in "0123456789abcdef"
                            for character in observation["prefix_digest"]
                        )
                    )
                )
            ):
                raise ValueError("qualification observation identity is invalid")
            if (
                not isinstance(repetition, int)
                or isinstance(observation["repetition"], bool)
                or not 0 <= repetition < repetitions
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
                timings[field_name] is not None
                and (
                    not isinstance(timings[field_name], float)
                    or not math.isfinite(timings[field_name])
                    or timings[field_name] < 0.0
                )
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
            if latency is not None and not (
                observation["accepted"] or observation["repaired"]
            ):
                raise ValueError("qualification observation latency is not reviewed")
            if observation["prefix_reused"] and observation["prefix_invalidated"]:
                raise ValueError("qualification observation prefix flags are invalid")
            if observation_identity_complete:
                prefix_digest = observation["prefix_digest"]
                if prefix_digest:
                    if previous_prefix_digest is not None:
                        expected_reused = prefix_digest == previous_prefix_digest
                        expected_invalidated = not expected_reused
                        if (
                            observation["prefix_reused"] != expected_reused
                            or observation["prefix_invalidated"] != expected_invalidated
                        ):
                            raise ValueError(
                                "qualification observation prefix semantics are invalid"
                            )
                    previous_prefix_digest = prefix_digest
                elif observation["prefix_reused"] or observation["prefix_invalidated"]:
                    raise ValueError(
                        "qualification observation prefix semantics are invalid"
                    )
            canonical_fixture = canonical_fixtures.get(fixture_id)
            if canonical_fixture is not None:
                expected = observation["outcome"] in canonical_fixture.expected_outcomes
                quality_values = [
                    observation[field_name]
                    for field_name in canonical_fixture.quality_fields
                ]
                control_error = observation["error_code"] in {
                    "qualification-cancelled",
                    "qualification-deadline-exceeded",
                    "context-over-budget",
                    "runner-failed",
                    "invalid-runner-result",
                }
                if canonical_fixture.kind == "cancellation" and observation[
                    "error_code"
                ] in {
                    "qualification-cancelled",
                    "qualification-deadline-exceeded",
                }:
                    control_error = False
                expected_reliable = (
                    expected
                    and not observation["context_over_budget"]
                    and not control_error
                    and all(quality_values)
                )
                if observation["outcome"] in {"accepted", "repaired"}:
                    expected_reliable = (
                        expected_reliable
                        and observation["reviewed_latency_ms"] is not None
                    )
                if (
                    observation["accepted"] != (observation["outcome"] == "accepted")
                    or observation["escalated"]
                    != (observation["outcome"] == "escalated")
                    or observation["cancelled"]
                    != (observation["outcome"] == "cancelled")
                    or observation["repaired"]
                    != (
                        observation["outcome"] == "repaired"
                        or (
                            canonical_fixture.kind == "repair"
                            and observation["outcome"] == "accepted"
                            and "accepted" in canonical_fixture.expected_outcomes
                        )
                    )
                    or observation["policy_blocked"]
                    != (
                        observation["outcome"] == "policy-blocked"
                        or (
                            canonical_fixture.kind == "policy-violation"
                            and observation["outcome"] == "escalated"
                            and "escalated" in canonical_fixture.expected_outcomes
                        )
                    )
                    or observation["expected"] != expected
                    or observation["reliable"] != expected_reliable
                    or observation["quality_score"]
                    != round(sum(quality_values) / len(quality_values), 4)
                ):
                    raise ValueError("qualification observation semantics are invalid")
        if any(values != set(range(repetitions)) for values in counts.values()):
            raise ValueError("qualification fixture repetitions are incomplete")

    @classmethod
    def _observation(
        cls,
        fixture: GovernedFixture,
        profile: LocalInferenceProfile,
        result: Mapping[str, Any],
        *,
        repetition: int,
        runtime_pin: RuntimePin,
        context: Mapping[str, Any] | None = None,
    ) -> dict[str, Any]:
        outcome = str(result["outcome"])
        route = result.get("route")
        plan = result.get("plan")
        evidence = result.get("evidence")
        context_tokens = int((context or {}).get("token_count", 0))
        cancellation_outcome = outcome == "cancelled" and result.get(
            "error_code", ""
        ) in {
            "",
            "qualification-cancelled",
            "qualification-deadline-exceeded",
        }
        flags = {
            "route_valid": isinstance(route, Mapping)
            and str(route.get("kind", "")) == _ROUTE_KINDS[fixture.kind],
            "plan_valid": fixture.kind in _PLAN_KINDS
            and isinstance(plan, Mapping)
            and bool(plan),
            "evidence_valid": fixture.kind in _EVIDENCE_KINDS
            and isinstance(evidence, Mapping)
            and bool(evidence),
            "accepted": outcome == "accepted",
            "repaired": outcome == "repaired"
            or (
                fixture.kind == "repair"
                and outcome == "accepted"
                and "accepted" in fixture.expected_outcomes
            ),
            "escalated": outcome == "escalated",
            "policy_blocked": outcome == "policy-blocked"
            or (
                fixture.kind == "policy-violation"
                and outcome == "escalated"
                and "escalated" in fixture.expected_outcomes
            ),
            "cancelled": outcome == "cancelled",
            "model_swapped": bool(result.get("model_swapped", False)),
            "queued": bool(result.get("queued", False)),
            "context_over_budget": bool(result.get("context_over_budget", False))
            or bool((context or {}).get("context_over_budget", False))
            or (
                not cancellation_outcome
                and context_tokens < fixture.required_context_tokens
            ),
        }
        quality_values = [flags[field_name] for field_name in fixture.quality_fields]
        expected = outcome in fixture.expected_outcomes
        reliable = expected and not bool(result.get("unexpected_failure", False))
        reliable = reliable and not flags["context_over_budget"]
        reliable = reliable and all(quality_values)
        latency = result.get("reviewed_latency_ms")
        if not (flags["accepted"] or flags["repaired"]):
            latency = None
        if flags["accepted"] or flags["repaired"]:
            reliable = reliable and latency is not None
        return {
            "fixture_id": fixture.fixture_id,
            "fixture_kind": fixture.kind,
            "repetition": repetition,
            "profile_id": profile.profile_id,
            "runtime_pin": runtime_pin.to_dict(),
            "model_digest": str(result.get("model_digest", "")),
            **flags,
            "outcome": outcome,
            "expected": expected,
            "reliable": reliable,
            "quality_score": round(sum(quality_values) / len(quality_values), 4),
            "timings": dict(result["timings"]),
            "reviewed_latency_ms": latency,
            "error_code": str(result.get("error_code", "")),
            "context_source_digest": str((context or {}).get("source_digest", "")),
            "context_cache_hit": bool((context or {}).get("cache_hit", False)),
            "prefix_digest": str((context or {}).get("prefix_digest", "")),
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
            and (item["accepted"] or item["repaired"])
        ]
        timings = {
            field_name: cls._summary(
                [
                    float(item["timings"][field_name])
                    for item in observations
                    if item["timings"][field_name] is not None
                ]
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
        rollback_receipt_id: str,
        rollback_evidence: Mapping[str, Any],
        baseline_report: QualificationReport | None,
        fixture_ids: tuple[str, ...],
        fixture_digests: Mapping[str, str],
        fixture_digests_complete: bool,
        repetitions: int,
    ) -> list[str]:
        blockers: list[str] = []
        if not fixture_digests_complete:
            blockers.append("fixture-digests-incomplete")
        if any(
            "runtime_pin" not in item or "prefix_digest" not in item
            for item in observations
        ):
            blockers.append("observation-identity-incomplete")
        canonical_fixture_ids = {
            fixture.fixture_id for fixture in build_governed_fixture_family()
        }
        if set(fixture_ids) != canonical_fixture_ids:
            blockers.append("fixture-coverage-incomplete")
        if repetitions < _MIN_QUALIFICATION_REPETITIONS:
            blockers.append("repetition-count-insufficient")
        if not profile.qualified:
            blockers.append("profile-not-qualified")
        if profile.model_digest in {"", "auto"}:
            blockers.append("exact-model-digest-required")
        elif any(
            item.get("model_digest") != profile.model_digest for item in observations
        ):
            blockers.append("model-digest-mismatch")
        if not observations or any(not item["reliable"] for item in observations):
            blockers.append("reliability-not-qualified")
        if metrics["quality_rate"] < 1.0:
            blockers.append("quality-not-qualified")
        if metrics["unexpected_outcomes"]:
            blockers.append("unexpected-outcome")
        if not rollback_tested or not rollback_receipt_id or not rollback_evidence:
            blockers.append("rollback-not-tested")
        if runtime_pin.withdrawn:
            blockers.append("runtime-withdrawn")
        if baseline_report is None:
            blockers.append("baseline-required")
        else:
            if (
                set(baseline_report.fixture_ids) != set(fixture_ids)
                or baseline_report.fixture_digests != dict(fixture_digests)
                or not baseline_report.fixture_digests_complete
                or baseline_report.repetitions != repetitions
                or baseline_report.runtime_pin != runtime_pin.to_dict()
                or _baseline_profile_identity(baseline_report.profile)
                != _baseline_profile_identity(profile.to_dict())
            ):
                blockers.append("baseline-incomparable")
            if metrics["quality_rate"] < baseline_report.metrics["quality_rate"]:
                blockers.append("quality-regression")
            if (
                metrics["reliability_rate"]
                < baseline_report.metrics["reliability_rate"]
            ):
                blockers.append("reliability-regression")
        return blockers
