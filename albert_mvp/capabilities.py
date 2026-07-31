from __future__ import annotations

import ast
from dataclasses import asdict, dataclass
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import socket
from typing import Any, Callable, Sequence
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from .agents import AgentConfig, AgentRegistry, is_eligible_assignment_agent


WORKSPACE_SKILL_ROOTS = (
    Path(".agents/skills"),
    Path(".agent/skills"),
    Path(".codex/skills"),
)
SKILL_NAME_PATTERN = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*\Z")
SKILL_METADATA_BYTES_LIMIT = 64 * 1024
SKILL_DISCOVERY_ENTRY_LIMIT = 20_000
SKILL_DISCOVERY_MATCH_LIMIT = 1_024
DEFAULT_OLLAMA_HOST = "http://127.0.0.1:11434"
OLLAMA_HEALTH_TIMEOUT_SECONDS = 0.15
OLLAMA_TAGS_RESPONSE_LIMIT = 1_000_000


@dataclass(frozen=True)
class SkillCapability:
    name: str
    description: str
    source: str
    invocation: str


@dataclass(frozen=True)
class AgentCapability:
    id: str
    role: str
    provider: str
    runner: str
    model: str
    routing: str
    availability: str
    availability_reason: str
    assignable: bool
    delegate_only: bool
    requires_approval: bool


@dataclass(frozen=True)
class OllamaHealthSnapshot:
    reachable: bool
    models: frozenset[str] = frozenset()
    availability_reason: str = ""


@dataclass
class _SkillDiscoveryBudget:
    entries_remaining: int = SKILL_DISCOVERY_ENTRY_LIMIT
    matches_remaining: int = SKILL_DISCOVERY_MATCH_LIMIT


class OllamaHealthProbe:
    """Fetch Ollama's local model catalog through one short, bounded request."""

    def __init__(
        self,
        *,
        host: str | None = None,
        timeout_seconds: float = OLLAMA_HEALTH_TIMEOUT_SECONDS,
        opener: Callable[..., Any] | None = None,
    ) -> None:
        self.host = host
        self.timeout_seconds = timeout_seconds
        self.opener = opener or urlopen

    def __call__(self) -> OllamaHealthSnapshot:
        endpoint = _ollama_tags_endpoint(self.host)
        request = Request(
            endpoint,
            headers={"Accept": "application/json"},
            method="GET",
        )
        try:
            with self.opener(request, timeout=self.timeout_seconds) as response:
                payload = response.read(OLLAMA_TAGS_RESPONSE_LIMIT + 1)
            if len(payload) > OLLAMA_TAGS_RESPONSE_LIMIT:
                raise ValueError("response exceeded the size limit")
            document = json.loads(payload.decode("utf-8"))
            models = _ollama_models_from_document(document)
        except (TimeoutError, socket.timeout):
            return OllamaHealthSnapshot(
                reachable=False,
                availability_reason=(
                    "Ollama health check timed out after "
                    f"{self.timeout_seconds:.3f}s at {endpoint}. "
                    "Start Ollama or set OLLAMA_HOST to the active daemon."
                ),
            )
        except HTTPError as error:
            return OllamaHealthSnapshot(
                reachable=False,
                availability_reason=(
                    f"Ollama returned HTTP {error.code} at {endpoint}. "
                    "Start Ollama or check OLLAMA_HOST and the daemon logs."
                ),
            )
        except URLError as error:
            if isinstance(error.reason, (TimeoutError, socket.timeout)):
                return OllamaHealthSnapshot(
                    reachable=False,
                    availability_reason=(
                        "Ollama health check timed out after "
                        f"{self.timeout_seconds:.3f}s at {endpoint}. "
                        "Start Ollama or set OLLAMA_HOST to the active daemon."
                    ),
                )
            return OllamaHealthSnapshot(
                reachable=False,
                availability_reason=(
                    f"Ollama is unreachable at {endpoint} ({error.reason}). "
                    "Start Ollama or set OLLAMA_HOST to the active daemon."
                ),
            )
        except (UnicodeError, ValueError, TypeError) as error:
            return OllamaHealthSnapshot(
                reachable=False,
                availability_reason=(
                    f"Ollama returned an invalid response at {endpoint} ({error}). "
                    "Start Ollama or check OLLAMA_HOST and the daemon logs."
                ),
            )
        except OSError as error:
            return OllamaHealthSnapshot(
                reachable=False,
                availability_reason=(
                    f"Ollama health check failed at {endpoint} ({error}). "
                    "Start Ollama or check OLLAMA_HOST and the daemon logs."
                ),
            )
        return OllamaHealthSnapshot(reachable=True, models=frozenset(models))


@dataclass(frozen=True)
class CommandCapability:
    name: str
    usage: str
    description: str
    category: str


BUILTIN_COMMANDS = (
    CommandCapability(
        name="/help",
        usage="/help",
        description="Show available commands and invocation help.",
        category="discovery",
    ),
    CommandCapability(
        name="/skills",
        usage="/skills [query]",
        description="Browse installed skills, optionally filtered by a query.",
        category="discovery",
    ),
    CommandCapability(
        name="/use",
        usage="/use <skill> [request]",
        description="Invoke an installed skill for the optional request.",
        category="agent",
    ),
    CommandCapability(
        name="/run",
        usage="/run <command>",
        description="Run a governed shell command in the active workspace.",
        category="execution",
    ),
    CommandCapability(
        name="/task",
        usage="/task <request>",
        description="Start or steer coding work with the selected controller.",
        category="agent",
    ),
    CommandCapability(
        name="/status",
        usage="/status",
        description="Show controller, subagent, and ready-work status.",
        category="monitoring",
    ),
)


@dataclass(frozen=True)
class CapabilityCatalogProjection:
    schema_version: int
    default_agent_id: str
    skills: list[SkillCapability]
    commands: list[CommandCapability]
    agents: list[AgentCapability]

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class CapabilityCatalogService:
    def __init__(
        self,
        *,
        workspace_root: Path,
        agent_registry: AgentRegistry,
        skill_roots: Sequence[Path] = (),
        global_skill_roots: Sequence[Path] | None = None,
        ollama_probe: Callable[[], OllamaHealthSnapshot] | None = None,
    ) -> None:
        self.workspace_root = workspace_root
        self.agent_registry = agent_registry
        self.skill_roots = tuple(skill_roots)
        self.global_skill_roots = (
            default_global_skill_roots()
            if global_skill_roots is None
            else tuple(global_skill_roots)
        )
        self.ollama_probe = ollama_probe or OllamaHealthProbe()

    def inspect(self) -> CapabilityCatalogProjection:
        skills: list[SkillCapability] = []
        seen_names: set[str] = set()
        discovery_budget = _SkillDiscoveryBudget(
            entries_remaining=SKILL_DISCOVERY_ENTRY_LIMIT,
            matches_remaining=SKILL_DISCOVERY_MATCH_LIMIT,
        )
        for root in self._skill_roots():
            for skill_file in _bounded_skill_files(root, discovery_budget):
                metadata = _read_skill_metadata(skill_file)
                if metadata is None:
                    continue
                name, description = metadata
                normalized_name = name.casefold()
                if normalized_name in seen_names:
                    continue
                seen_names.add(normalized_name)
                skills.append(
                    SkillCapability(
                        name=name,
                        description=description,
                        source=str(skill_file.resolve()),
                        invocation=f"/use {name}",
                    )
                )
        skills.sort(key=lambda skill: (skill.name.casefold(), skill.name))
        agents = self.agent_availability()
        controller = self.agent_registry.controller_agent()
        return CapabilityCatalogProjection(
            schema_version=1,
            default_agent_id=controller.id if controller is not None else "",
            skills=skills,
            commands=list(BUILTIN_COMMANDS),
            agents=agents,
        )

    def agent_availability(self) -> list[AgentCapability]:
        """Project one provider-health snapshot across every configured agent."""
        ollama_health = self._ollama_health()
        return [
            _project_agent(
                agent,
                workspace_root=self.workspace_root,
                ollama_health=ollama_health,
            )
            for agent in self.agent_registry.agents
        ]

    def _skill_roots(self) -> tuple[Path, ...]:
        workspace_roots = tuple(self.workspace_root / root for root in WORKSPACE_SKILL_ROOTS)
        candidates = workspace_roots + self.skill_roots + self.global_skill_roots
        roots: list[Path] = []
        seen: set[Path] = set()
        for candidate in candidates:
            expanded = candidate.expanduser()
            if expanded.is_symlink():
                continue
            resolved = expanded.resolve()
            if resolved in seen:
                continue
            seen.add(resolved)
            roots.append(resolved)
        return tuple(roots)

    def _ollama_health(self) -> OllamaHealthSnapshot | None:
        if not any(
            agent.availability == "available" and _uses_ollama(agent)
            for agent in self.agent_registry.agents
        ):
            return None
        try:
            health = self.ollama_probe()
        except Exception as error:
            return OllamaHealthSnapshot(
                reachable=False,
                availability_reason=(
                    f"Ollama health probe failed ({error}). "
                    "Start Ollama or check OLLAMA_HOST and the daemon logs."
                ),
            )
        if not isinstance(health, OllamaHealthSnapshot):
            return OllamaHealthSnapshot(
                reachable=False,
                availability_reason=(
                    "Ollama health probe returned an invalid result. "
                    "Start Ollama or check OLLAMA_HOST and the daemon logs."
                ),
            )
        return health


def default_global_skill_roots() -> tuple[Path, ...]:
    home = Path.home()
    configured_codex_home = os.environ.get("CODEX_HOME", "").strip()
    codex_home = (
        Path(configured_codex_home).expanduser()
        if configured_codex_home
        else home / ".codex"
    )
    return (
        home / ".agents" / "skills",
        codex_home / "skills",
        codex_home / "plugins" / "cache",
    )


def _bounded_skill_files(
    root: Path,
    budget: _SkillDiscoveryBudget,
) -> list[Path]:
    matches: list[Path] = []
    pending = [root]
    while (
        pending
        and budget.entries_remaining > 0
        and budget.matches_remaining > 0
    ):
        directory = pending.pop()
        try:
            entries = os.scandir(directory)
        except OSError:
            continue
        discovered_entries: list[os.DirEntry[str]] = []
        child_directories: list[Path] = []
        with entries:
            while budget.entries_remaining > 0:
                try:
                    entry = next(entries)
                except StopIteration:
                    break
                budget.entries_remaining -= 1
                discovered_entries.append(entry)
        for entry in sorted(discovered_entries, key=lambda candidate: candidate.name):
            try:
                if entry.is_symlink():
                    continue
                if entry.is_dir(follow_symlinks=False):
                    child_directories.append(Path(entry.path))
                elif (
                    entry.name == "SKILL.md"
                    and entry.is_file(follow_symlinks=False)
                ):
                    matches.append(Path(entry.path))
                    budget.matches_remaining -= 1
                    if budget.matches_remaining == 0:
                        break
            except OSError:
                continue
        pending.extend(
            reversed(sorted(child_directories, key=lambda path: path.name))
        )
    return sorted(matches)


def _read_skill_metadata(skill_file: Path) -> tuple[str, str] | None:
    frontmatter = _read_skill_frontmatter(skill_file)
    if frontmatter is None:
        return None
    metadata: dict[str, str] = {}
    index = 0
    while index < len(frontmatter):
        line = frontmatter[index]
        if ":" not in line:
            index += 1
            continue
        key, value = line.split(":", 1)
        key = key.strip()
        value = value.strip()
        if value in {">", ">-", ">+", "|", "|-", "|+"}:
            block_lines: list[str] = []
            index += 1
            while index < len(frontmatter):
                block_line = frontmatter[index]
                if block_line and not block_line[0].isspace():
                    break
                block_lines.append(block_line.strip())
                index += 1
            metadata[key] = _render_yaml_block(value[0], block_lines)
            continue
        metadata[key] = _parse_yaml_scalar(value)
        index += 1
    name = metadata.get("name", "").strip()
    description = metadata.get("description", "").strip()
    if not SKILL_NAME_PATTERN.fullmatch(name) or not description:
        return None
    return name, description


def _read_skill_frontmatter(skill_file: Path) -> list[str] | None:
    """Read only a bounded UTF-8 front matter prefix from one skill file."""

    frontmatter: list[str] = []
    bytes_read = 0
    try:
        with skill_file.open("rb") as source:
            while bytes_read <= SKILL_METADATA_BYTES_LIMIT:
                remaining = SKILL_METADATA_BYTES_LIMIT - bytes_read
                raw_line = source.readline(remaining + 1)
                if not raw_line:
                    return None
                bytes_read += len(raw_line)
                if bytes_read > SKILL_METADATA_BYTES_LIMIT:
                    return None
                line = raw_line.decode("utf-8").rstrip("\r\n")
                if not frontmatter:
                    if line.strip() != "---":
                        return None
                    frontmatter.append(line)
                    continue
                if line.strip() == "---":
                    return frontmatter[1:]
                frontmatter.append(line)
    except (OSError, UnicodeError):
        return None
    return None


def _project_agent(
    agent: AgentConfig,
    *,
    workspace_root: Path,
    ollama_health: OllamaHealthSnapshot | None,
) -> AgentCapability:
    availability, availability_reason = _agent_availability(
        agent,
        workspace_root=workspace_root,
        ollama_health=ollama_health,
    )
    return AgentCapability(
        id=agent.id,
        role=agent.role,
        provider=agent.provider,
        runner=agent.runner,
        model=agent.model,
        routing=agent.routing,
        availability=availability,
        availability_reason=availability_reason,
        assignable=is_eligible_assignment_agent(agent),
        delegate_only=agent.delegate_only,
        requires_approval=agent.requires_approval,
    )


def _agent_availability(
    agent: AgentConfig,
    *,
    workspace_root: Path,
    ollama_health: OllamaHealthSnapshot | None,
) -> tuple[str, str]:
    if agent.availability != "available":
        return agent.availability, agent.availability_reason
    if _uses_ollama(agent):
        if ollama_health is None or not ollama_health.reachable:
            reason = (
                ollama_health.availability_reason
                if ollama_health is not None
                else ""
            )
            return (
                "disconnected",
                reason
                or (
                    "Ollama is unavailable. Start Ollama or set OLLAMA_HOST "
                    "to the active daemon."
                ),
            )
        configured_model = _normalize_ollama_model(agent.model)
        installed_models = {
            _normalize_ollama_model(model) for model in ollama_health.models
        }
        if not configured_model or configured_model not in installed_models:
            model = agent.model or "<missing model>"
            return (
                "unavailable",
                f"Ollama model {model!r} is not installed. "
                f"Run `ollama pull {model}` and refresh capabilities.",
            )
        return "available", agent.availability_reason
    if agent.runner.casefold() == "command":
        return _command_agent_availability(agent.command, workspace_root)
    return "available", agent.availability_reason


def _uses_ollama(agent: AgentConfig) -> bool:
    return agent.runner.casefold() == "ollama"


def _command_agent_availability(command: str, workspace_root: Path) -> tuple[str, str]:
    try:
        argv = shlex.split(command)
    except ValueError as error:
        return (
            "unavailable",
            f"The configured agent command is invalid ({error}). Update the agent command.",
        )
    if not argv:
        return "unavailable", "No agent command is configured. Update the agent command."
    executable = argv[0]
    if os.sep in executable or (os.altsep is not None and os.altsep in executable):
        executable_path = Path(executable).expanduser()
        if not executable_path.is_absolute():
            executable_path = workspace_root / executable_path
        executable_available = executable_path.is_file() and os.access(
            executable_path,
            os.X_OK,
        )
    else:
        executable_available = shutil.which(executable) is not None
    if executable_available:
        return "available", ""
    return (
        "unavailable",
        f"Command runner executable {executable!r} was not found or is not executable. "
        "Install it or update the agent command.",
    )


def _ollama_tags_endpoint(configured_host: str | None) -> str:
    host = (
        configured_host
        if configured_host is not None
        else os.environ.get("OLLAMA_HOST", "")
    ).strip()
    if not host:
        host = DEFAULT_OLLAMA_HOST
    if "://" not in host:
        host = f"http://{host}"
    return f"{host.rstrip('/')}/api/tags"


def _ollama_models_from_document(document: object) -> set[str]:
    if not isinstance(document, dict) or not isinstance(document.get("models"), list):
        raise ValueError("expected a JSON object containing a models list")
    models: set[str] = set()
    for item in document["models"]:
        if not isinstance(item, dict):
            continue
        for field_name in ("name", "model"):
            value = item.get(field_name)
            if isinstance(value, str) and value.strip():
                models.add(_normalize_ollama_model(value))
    return models


def _normalize_ollama_model(model: str) -> str:
    normalized = model.strip().casefold()
    if normalized and ":" not in normalized.rsplit("/", 1)[-1]:
        normalized = f"{normalized}:latest"
    return normalized


def _parse_yaml_scalar(value: str) -> str:
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
        try:
            parsed = ast.literal_eval(value)
        except (SyntaxError, ValueError):
            return value[1:-1]
        if isinstance(parsed, str):
            return parsed
    return value


def _render_yaml_block(style: str, lines: list[str]) -> str:
    if style == "|":
        return "\n".join(lines).strip()
    paragraphs: list[str] = []
    current: list[str] = []
    for line in lines:
        if line:
            current.append(line)
            continue
        if current:
            paragraphs.append(" ".join(current))
            current = []
    if current:
        paragraphs.append(" ".join(current))
    return "\n".join(paragraphs)
