from __future__ import annotations

from dataclasses import dataclass, field
import json
from pathlib import Path
from typing import Any

from .inference import LocalInferenceProfile


class AgentConfigError(Exception):
    """Raised when the local agent registry config is invalid."""


def is_cloud_model(model: str) -> bool:
    """Return whether a configured model uses the governed cloud suffix."""

    return model.strip().casefold().endswith(":cloud")


_LOCAL_EXECUTION_PROVIDERS = frozenset(
    {"command", "fake", "local", "ollama", "test", "test-harness"}
)
_LOCAL_EXECUTION_RUNNERS = frozenset({"command", "fake", "ollama"})


@dataclass(frozen=True)
class AgentConfig:
    id: str
    role: str
    provider: str
    runner: str
    model: str = ""
    command: str = ""
    test_command: str = ""
    routing: str = ""
    assignable: bool = True
    delegate_only: bool = False
    requires_approval: bool = False
    availability: str = "available"
    availability_reason: str = ""
    inference_profile: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        data: dict[str, Any] = {
            "id": self.id,
            "role": self.role,
            "provider": self.provider,
            "runner": self.runner,
        }
        if self.model:
            data["model"] = self.model
        if self.command:
            data["command"] = self.command
        if self.test_command:
            data["test_command"] = self.test_command
        if self.routing:
            data["routing"] = self.routing
        if not self.assignable:
            data["assignable"] = False
        if self.delegate_only:
            data["delegate_only"] = self.delegate_only
        if self.requires_approval:
            data["requires_approval"] = self.requires_approval
        if self.availability != "available":
            data["availability"] = self.availability
        if self.availability_reason:
            data["availability_reason"] = self.availability_reason
        if self.inference_profile:
            data["inference_profile"] = json.loads(
                json.dumps(self.inference_profile, ensure_ascii=True)
            )
        return data

    def summary(self) -> str:
        prefix = "delegate-only " if self.delegate_only else ""
        if self.runner == "command":
            return f"{prefix}{self.runner}:{self.command}"
        if self.model:
            return f"{prefix}{self.provider}:{self.model}"
        return prefix + (self.provider or self.runner)


def has_local_execution_boundary(agent: AgentConfig) -> bool:
    """Return whether both provider and runner are explicitly local-capable."""

    return (
        agent.provider.strip().casefold() in _LOCAL_EXECUTION_PROVIDERS
        and agent.runner.strip().casefold() in _LOCAL_EXECUTION_RUNNERS
        and not is_cloud_model(agent.model)
    )


def is_eligible_assignment_agent(agent: AgentConfig) -> bool:
    """Return whether an agent may be selected as an ordinary Local Agent worker."""

    routing = agent.routing.strip().casefold()
    has_worker_role = routing == "worker" or (
        not routing and agent.role.strip().casefold() == "local-agent"
    )
    return (
        has_worker_role
        and agent.assignable
        and not agent.delegate_only
        and not agent.requires_approval
        and has_local_execution_boundary(agent)
    )


def is_eligible_controller_agent(agent: AgentConfig) -> bool:
    """Return whether an agent may act as a local controller or router."""

    routing = agent.routing.strip().casefold()
    has_controller_role = routing in {"controller", "router", "frontier"} or (
        not routing and agent.role.strip().casefold() == "frontier"
    )
    return (
        has_controller_role
        and not agent.delegate_only
        and not agent.requires_approval
        and has_local_execution_boundary(agent)
        and agent.availability.strip().casefold() == "available"
    )


@dataclass(frozen=True)
class AgentRegistry:
    agents: list[AgentConfig]
    source_path: Path | None = None

    @property
    def configured(self) -> bool:
        return bool(self.agents)

    def find(self, agent_id: str) -> AgentConfig | None:
        for agent in self.agents:
            if agent.id == agent_id:
                return agent
        return None

    def require(self, agent_id: str) -> AgentConfig:
        agent = self.find(agent_id)
        if not agent:
            raise AgentConfigError(f"Unknown configured agent: {agent_id}")
        return agent

    def controller_agent(self) -> AgentConfig | None:
        for routing in ("controller", "router", "frontier"):
            for agent in self.agents:
                if (
                    agent.routing.casefold() == routing
                    and is_eligible_controller_agent(agent)
                ):
                    return agent
        for agent in self.agents:
            if (
                not agent.routing.strip()
                and agent.role.casefold() == "frontier"
                and is_eligible_controller_agent(agent)
            ):
                return agent
        return None


def load_agent_registry(path: Path | None) -> AgentRegistry:
    if path is None or not path.exists():
        return AgentRegistry(agents=[], source_path=path)
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise AgentConfigError(f"{path} must contain a JSON object.")
    raw_agents = data.get("agents", [])
    if not isinstance(raw_agents, list):
        raise AgentConfigError(f"{path} field 'agents' must be a list.")
    agents: list[AgentConfig] = []
    seen: set[str] = set()
    for index, raw_agent in enumerate(raw_agents, start=1):
        agents.append(_parse_agent(path, index, raw_agent, seen))
    return AgentRegistry(agents=agents, source_path=path)


def _parse_agent(path: Path, index: int, raw_agent: Any, seen: set[str]) -> AgentConfig:
    if not isinstance(raw_agent, dict):
        raise AgentConfigError(f"{path} agent entry {index} must be a JSON object.")
    agent_id = _required(raw_agent, "id", path, index)
    if agent_id in seen:
        raise AgentConfigError(f"{path} agent entry {agent_id} duplicates an existing id.")
    seen.add(agent_id)
    role = _required(raw_agent, "role", path, agent_id)
    runner = _required(raw_agent, "runner", path, agent_id)
    provider = str(raw_agent.get("provider", runner)).strip()
    if not provider:
        raise AgentConfigError(f"{path} agent entry {agent_id} is missing provider.")
    model = str(raw_agent.get("model", "")).strip()
    command = str(raw_agent.get("command", "")).strip()
    test_command = str(raw_agent.get("test_command", "")).strip()
    routing = str(raw_agent.get("routing", "")).strip()
    assignable = _optional_bool(raw_agent, "assignable", True, path, agent_id)
    delegate_only = _optional_bool(raw_agent, "delegate_only", False, path, agent_id)
    requires_approval = _optional_bool(
        raw_agent,
        "requires_approval",
        False,
        path,
        agent_id,
    )
    availability = str(raw_agent.get("availability", "available")).strip() or "available"
    availability_reason = str(raw_agent.get("availability_reason", "")).strip()
    raw_inference_profile = raw_agent.get("inference_profile", {})
    if not isinstance(raw_inference_profile, dict):
        raise AgentConfigError(
            f"{path} agent entry {agent_id} field 'inference_profile' must be an object."
        )
    if availability not in {"available", "unavailable", "disconnected"}:
        raise AgentConfigError(f"{path} agent entry {agent_id} has unknown availability {availability!r}.")
    if runner == "command" and not command:
        raise AgentConfigError(f"{path} agent entry {agent_id} is missing command.")
    if runner == "ollama" and not model:
        raise AgentConfigError(f"{path} agent entry {agent_id} is missing model.")
    inference_profile: dict[str, Any] = {}
    if raw_inference_profile:
        try:
            inference_profile = LocalInferenceProfile.from_dict(
                raw_inference_profile,
                model=model,
                profile_id=f"{agent_id}-v1",
            ).to_dict()
        except ValueError as exc:
            raise AgentConfigError(
                f"{path} agent entry {agent_id} has an invalid inference_profile: {exc}"
            ) from exc
    return AgentConfig(
        id=agent_id,
        role=role,
        provider=provider,
        runner=runner,
        model=model,
        command=command,
        test_command=test_command,
        routing=routing,
        assignable=assignable,
        delegate_only=delegate_only,
        requires_approval=requires_approval,
        availability=availability,
        availability_reason=availability_reason,
        inference_profile=inference_profile,
    )


def _required(raw_agent: dict[str, Any], field_name: str, path: Path, label: object) -> str:
    value = str(raw_agent.get(field_name, "")).strip()
    if not value:
        raise AgentConfigError(f"{path} agent entry {label} is missing {field_name}.")
    return value


def _optional_bool(
    raw_agent: dict[str, Any],
    field_name: str,
    default: bool,
    path: Path,
    label: object,
) -> bool:
    value = raw_agent.get(field_name, default)
    if not isinstance(value, bool):
        raise AgentConfigError(
            f"{path} agent entry {label} field {field_name!r} must be a JSON boolean."
        )
    return value
