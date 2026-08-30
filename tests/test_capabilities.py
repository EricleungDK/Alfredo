from __future__ import annotations

import json
import io
import os
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from urllib.error import URLError
from unittest.mock import patch

from albert_mvp.agents import AgentConfig, AgentRegistry, is_cloud_model
from albert_mvp.capabilities import (
    CapabilityCatalogService,
    OllamaHealthProbe,
    OllamaHealthSnapshot,
)
from albert_mvp.cli import main


class FakeHttpResponse:
    def __init__(self, payload: bytes) -> None:
        self.payload = payload

    def __enter__(self) -> FakeHttpResponse:
        return self

    def __exit__(self, *args: object) -> None:
        return None

    def read(self, _limit: int = -1) -> bytes:
        return self.payload


class CapabilityCatalogTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write_skill(
        self,
        root: Path,
        directory_name: str,
        *,
        name: str,
        description: str,
    ) -> Path:
        skill_file = root / directory_name / "SKILL.md"
        skill_file.parent.mkdir(parents=True, exist_ok=True)
        skill_file.write_text(
            f"---\nname: {name}\ndescription: {description}\n---\n",
            encoding="utf-8",
        )
        return skill_file

    @staticmethod
    def healthy_ollama_probe(*models: str):
        return lambda: OllamaHealthSnapshot(
            reachable=True,
            models=frozenset(models),
        )

    def test_ollama_probe_projects_present_model_as_available_and_respects_host(self) -> None:
        requests: list[tuple[str, float]] = []

        def open_tags(request: object, *, timeout: float) -> FakeHttpResponse:
            requests.append((getattr(request, "full_url"), timeout))
            return FakeHttpResponse(
                json.dumps(
                    {
                        "models": [
                            {"name": "qwen3:14b"},
                            {"model": "worker:latest"},
                        ]
                    }
                ).encode("utf-8")
            )

        agent = AgentConfig(
            id="controller",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="qwen3:14b",
            routing="controller",
        )
        worker = AgentConfig(
            id="worker",
            role="local-agent",
            provider="ollama",
            runner="ollama",
            model="worker",
        )
        with patch.dict(os.environ, {"OLLAMA_HOST": "127.0.0.1:22434"}):
            projection = CapabilityCatalogService(
                workspace_root=self.workspace,
                agent_registry=AgentRegistry(agents=[agent, worker]),
                global_skill_roots=[],
                ollama_probe=OllamaHealthProbe(
                    timeout_seconds=0.05,
                    opener=open_tags,
                ),
            ).inspect()

        self.assertEqual(requests, [("http://127.0.0.1:22434/api/tags", 0.05)])
        self.assertEqual(
            [agent.availability for agent in projection.agents],
            ["available", "available"],
        )
        self.assertEqual(projection.agents[0].availability_reason, "")

    def test_ollama_probe_projects_missing_model_as_unavailable_with_pull_action(self) -> None:
        agent = AgentConfig(
            id="worker",
            role="local-agent",
            provider="ollama",
            runner="ollama",
            model="missing-model:latest",
        )

        projection = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[agent]),
            global_skill_roots=[],
            ollama_probe=self.healthy_ollama_probe("another-model:latest"),
        ).inspect()

        capability = projection.agents[0]
        self.assertEqual(capability.availability, "unavailable")
        self.assertIn("missing-model:latest", capability.availability_reason)
        self.assertIn("ollama pull missing-model:latest", capability.availability_reason)
        self.assertTrue(capability.assignable)

    def test_ollama_probe_projects_unreachable_daemon_as_disconnected(self) -> None:
        def refuse_connection(_request: object, *, timeout: float) -> FakeHttpResponse:
            del timeout
            raise URLError(ConnectionRefusedError("connection refused"))

        agent = AgentConfig(
            id="controller",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="qwen3:14b",
            routing="controller",
        )
        projection = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[agent]),
            global_skill_roots=[],
            ollama_probe=OllamaHealthProbe(
                host="http://127.0.0.1:33434",
                timeout_seconds=0.01,
                opener=refuse_connection,
            ),
        ).inspect()

        capability = projection.agents[0]
        self.assertEqual(capability.availability, "disconnected")
        self.assertIn("127.0.0.1:33434", capability.availability_reason)
        self.assertIn("Start Ollama", capability.availability_reason)
        self.assertIn("OLLAMA_HOST", capability.availability_reason)

    def test_ollama_probe_turns_timeout_and_invalid_response_into_actionable_health(self) -> None:
        def time_out(_request: object, *, timeout: float) -> FakeHttpResponse:
            del timeout
            raise TimeoutError("timed out")

        def invalid_json(_request: object, *, timeout: float) -> FakeHttpResponse:
            del timeout
            return FakeHttpResponse(b"not json")

        for opener, expected_text in (
            (time_out, "timed out"),
            (invalid_json, "invalid response"),
        ):
            with self.subTest(expected_text=expected_text):
                health = OllamaHealthProbe(
                    host="http://127.0.0.1:11434",
                    timeout_seconds=0.01,
                    opener=opener,
                )()
                self.assertFalse(health.reachable)
                self.assertIn(expected_text, health.availability_reason)
                self.assertIn("Start Ollama", health.availability_reason)

    def test_explicit_registry_unavailability_is_preserved_without_a_probe(self) -> None:
        def unexpected_probe() -> OllamaHealthSnapshot:
            raise AssertionError("explicitly unavailable agents must not trigger a probe")

        agent = AgentConfig(
            id="offline-worker",
            role="local-agent",
            provider="ollama",
            runner="ollama",
            model="worker:latest",
            availability="unavailable",
            availability_reason="Disabled by the operator.",
        )
        capability = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[agent]),
            global_skill_roots=[],
            ollama_probe=unexpected_probe,
        ).inspect().agents[0]

        self.assertEqual(capability.availability, "unavailable")
        self.assertEqual(capability.availability_reason, "Disabled by the operator.")

    def test_fake_and_command_agents_use_fast_local_availability_checks(self) -> None:
        executable = self.workspace / "local-agent"
        executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        executable.chmod(0o755)
        agents = [
            AgentConfig(
                id="fake-worker",
                role="local-agent",
                provider="fake",
                runner="fake",
            ),
            AgentConfig(
                id="command-worker",
                role="local-agent",
                provider="ollama",
                runner="command",
                model="wrapped-model:latest",
                command="./local-agent --task packet.json",
            ),
            AgentConfig(
                id="missing-command",
                role="local-agent",
                provider="command",
                runner="command",
                command="missing-agent-executable --task packet.json",
            ),
        ]

        capabilities = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=agents),
            global_skill_roots=[],
            ollama_probe=lambda: (_ for _ in ()).throw(
                AssertionError("non-Ollama catalog must not trigger an Ollama probe")
            ),
        ).inspect().agents

        self.assertEqual(
            [(agent.id, agent.availability) for agent in capabilities],
            [
                ("fake-worker", "available"),
                ("command-worker", "available"),
                ("missing-command", "unavailable"),
            ],
        )
        self.assertIn(
            "missing-agent-executable",
            capabilities[2].availability_reason,
        )
        self.assertIn("update the agent command", capabilities[2].availability_reason)

    def test_workspace_skill_metadata_is_discovered_as_json_ready_capability(self) -> None:
        skill_file = self.workspace / ".agents" / "skills" / "diagnose" / "SKILL.md"
        skill_file.parent.mkdir(parents=True)
        skill_file.write_text(
            "---\n"
            "name: diagnose\n"
            "description: Reproduce and isolate hard bugs.\n"
            "---\n\n"
            "# Diagnose\n",
            encoding="utf-8",
        )

        projection = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[]),
            global_skill_roots=[],
        ).inspect()

        payload = projection.to_dict()
        self.assertEqual(payload["schema_version"], 1)
        self.assertEqual(
            payload["skills"],
            [
                {
                    "name": "diagnose",
                    "description": "Reproduce and isolate hard bugs.",
                    "source": str(skill_file.resolve()),
                    "invocation": "/use diagnose",
                }
            ],
        )
        json.dumps(payload)

    def test_workspace_configured_and_global_roots_have_deterministic_precedence(self) -> None:
        workspace_root = self.workspace / ".codex" / "skills"
        configured_root = self.root / "configured-skills"
        global_root = self.root / "global-skills"
        workspace_shared = self.write_skill(
            workspace_root,
            "shared-workspace",
            name="shared",
            description="Workspace definition.",
        )
        self.write_skill(
            configured_root,
            "shared-configured",
            name="shared",
            description="Configured definition.",
        )
        self.write_skill(
            global_root,
            "shared-global",
            name="shared",
            description="Global definition.",
        )
        configured_only = self.write_skill(
            configured_root,
            "z-configured",
            name="configured-only",
            description="Configured capability.",
        )
        global_only = self.write_skill(
            global_root,
            "a-global",
            name="global-only",
            description="Global capability.",
        )

        payload = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[]),
            skill_roots=[configured_root],
            global_skill_roots=[global_root],
        ).inspect().to_dict()

        self.assertEqual(
            payload["skills"],
            [
                {
                    "name": "configured-only",
                    "description": "Configured capability.",
                    "source": str(configured_only.resolve()),
                    "invocation": "/use configured-only",
                },
                {
                    "name": "global-only",
                    "description": "Global capability.",
                    "source": str(global_only.resolve()),
                    "invocation": "/use global-only",
                },
                {
                    "name": "shared",
                    "description": "Workspace definition.",
                    "source": str(workspace_shared.resolve()),
                    "invocation": "/use shared",
                },
            ],
        )

    def test_malformed_metadata_is_skipped_while_folded_yaml_is_tolerated(self) -> None:
        skill_root = self.workspace / ".agents" / "skills"
        folded_skill = skill_root / "caveman" / "SKILL.md"
        folded_skill.parent.mkdir(parents=True)
        folded_skill.write_text(
            "---\n"
            'name: "caveman"\n'
            "description: >\n"
            "  Use fewer words while preserving\n"
            "  technical accuracy.\n"
            "---\n",
            encoding="utf-8",
        )
        missing_close = skill_root / "missing-close" / "SKILL.md"
        missing_close.parent.mkdir()
        missing_close.write_text(
            "---\nname: missing-close\ndescription: This never closes.\n",
            encoding="utf-8",
        )
        missing_name = skill_root / "missing-name" / "SKILL.md"
        missing_name.parent.mkdir()
        missing_name.write_text(
            "---\ndescription: No public name.\n---\n",
            encoding="utf-8",
        )
        invalid_utf8 = skill_root / "invalid-utf8" / "SKILL.md"
        invalid_utf8.parent.mkdir()
        invalid_utf8.write_bytes(b"---\nname: invalid\ndescription: \xff\n---\n")
        unsafe_name = skill_root / "unsafe-name" / "SKILL.md"
        unsafe_name.parent.mkdir()
        unsafe_name.write_text(
            "---\nname: diagnose --force\ndescription: Unsafe invocation text.\n---\n",
            encoding="utf-8",
        )

        payload = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[]),
            global_skill_roots=[],
        ).inspect().to_dict()

        self.assertEqual(
            payload["skills"],
            [
                {
                    "name": "caveman",
                    "description": "Use fewer words while preserving technical accuracy.",
                    "source": str(folded_skill.resolve()),
                    "invocation": "/use caveman",
                }
            ],
        )

    def test_skill_discovery_bounds_sparse_files_and_skips_oversized_metadata(self) -> None:
        skill_root = self.workspace / ".agents" / "skills"
        sparse_skill = skill_root / "sparse-valid" / "SKILL.md"
        sparse_skill.parent.mkdir(parents=True)
        with sparse_skill.open("wb") as output:
            output.write(
                b"---\n"
                b"name: sparse-valid\n"
                b"description: Discover metadata without reading the large body.\n"
                b"---\n"
            )
            output.truncate(16 * 1024 * 1024)

        oversized_skill = skill_root / "oversized-metadata" / "SKILL.md"
        oversized_skill.parent.mkdir()
        with oversized_skill.open("wb") as output:
            output.write(
                b"---\n"
                b"name: oversized-metadata\n"
                b"description: "
            )
            output.truncate(16 * 1024 * 1024)

        ordinary_skill = self.write_skill(
            skill_root,
            "ordinary",
            name="ordinary",
            description="Discovery continues after malformed metadata.",
        )
        original_open = Path.open
        guarded_paths = {sparse_skill, oversized_skill}
        bounded_read_sizes: dict[Path, list[int]] = {
            sparse_skill: [],
            oversized_skill: [],
        }

        class BoundedReadGuard:
            def __init__(self, path: Path, handle: object) -> None:
                self.path = path
                self.handle = handle

            def _record(self, size: int | None) -> None:
                if size is None or size < 0:
                    raise AssertionError("skill discovery must not read a whole skill file")
                if size > 65_537:
                    raise AssertionError("skill metadata reads must stay within 64 KiB")
                bounded_read_sizes[self.path].append(size)

            def read(self, size: int = -1) -> object:
                self._record(size)
                return self.handle.read(size)  # type: ignore[attr-defined]

            def readline(self, size: int = -1) -> object:
                self._record(size)
                return self.handle.readline(size)  # type: ignore[attr-defined]

            def __enter__(self) -> BoundedReadGuard:
                self.handle.__enter__()  # type: ignore[attr-defined]
                return self

            def __exit__(self, *args: object) -> object:
                return self.handle.__exit__(*args)  # type: ignore[attr-defined]

            def __getattr__(self, name: str) -> object:
                return getattr(self.handle, name)

        def guarded_open(path: Path, mode: str = "r", *args: object, **kwargs: object):
            handle = original_open(path, mode, *args, **kwargs)
            if path in guarded_paths and "r" in mode:
                return BoundedReadGuard(path, handle)
            return handle

        with patch.object(Path, "open", guarded_open):
            payload = CapabilityCatalogService(
                workspace_root=self.workspace,
                agent_registry=AgentRegistry(agents=[]),
                global_skill_roots=[],
            ).inspect().to_dict()

        self.assertEqual(
            payload["skills"],
            [
                {
                    "name": "ordinary",
                    "description": "Discovery continues after malformed metadata.",
                    "source": str(ordinary_skill.resolve()),
                    "invocation": "/use ordinary",
                },
                {
                    "name": "sparse-valid",
                    "description": "Discover metadata without reading the large body.",
                    "source": str(sparse_skill.resolve()),
                    "invocation": "/use sparse-valid",
                },
            ],
        )
        self.assertTrue(bounded_read_sizes[sparse_skill])
        self.assertTrue(bounded_read_sizes[oversized_skill])

    def test_skill_discovery_does_not_follow_symlinked_skill_files(self) -> None:
        skill_root = self.workspace / ".agents" / "skills"
        external_skill = self.write_skill(
            self.root / "outside-skills",
            "external",
            name="external-skill",
            description="Must not cross the configured discovery root.",
        )
        linked_skill = skill_root / "linked" / "SKILL.md"
        linked_skill.parent.mkdir(parents=True)
        linked_skill.symlink_to(external_skill)
        ordinary_skill = self.write_skill(
            skill_root,
            "ordinary",
            name="ordinary",
            description="A regular skill remains discoverable.",
        )

        skills = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[]),
            global_skill_roots=[],
        ).inspect().to_dict()["skills"]

        self.assertEqual(
            skills,
            [
                {
                    "name": "ordinary",
                    "description": "A regular skill remains discoverable.",
                    "source": str(ordinary_skill.resolve()),
                    "invocation": "/use ordinary",
                }
            ],
        )

    def test_skill_discovery_counts_every_directory_entry_toward_its_scan_cap(self) -> None:
        skill_root = self.workspace / ".agents" / "skills"
        deep_root = skill_root
        for level in range(4):
            deep_root = deep_root / f"level-{level}"
            deep_root.mkdir(parents=True)
        self.write_skill(
            deep_root,
            "too-deep",
            name="too-deep",
            description="This match is beyond the directory-entry budget.",
        )

        with patch("albert_mvp.capabilities.SKILL_DISCOVERY_ENTRY_LIMIT", 3):
            skills = CapabilityCatalogService(
                workspace_root=self.workspace,
                agent_registry=AgentRegistry(agents=[]),
                global_skill_roots=[],
            ).inspect().skills

        self.assertEqual(skills, [])

    def test_skill_discovery_stops_after_its_bounded_match_cap(self) -> None:
        skill_root = self.workspace / ".agents" / "skills"
        for name in ("alpha", "bravo", "charlie"):
            self.write_skill(
                skill_root,
                name,
                name=name,
                description=f"{name.title()} bounded capability.",
            )

        with patch("albert_mvp.capabilities.SKILL_DISCOVERY_MATCH_LIMIT", 2):
            skills = CapabilityCatalogService(
                workspace_root=self.workspace,
                agent_registry=AgentRegistry(agents=[]),
                global_skill_roots=[],
            ).inspect().skills

        self.assertEqual(len(skills), 2)
        self.assertTrue({skill.name for skill in skills}.issubset(
            {"alpha", "bravo", "charlie"}
        ))

    def test_configured_agents_are_projected_and_controller_is_the_default(self) -> None:
        frontier = AgentConfig(
            id="frontier",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="frontier-model",
        )
        router = AgentConfig(
            id="router",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="router-model",
            routing="router",
        )
        controller = AgentConfig(
            id="controller",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="controller-model",
            routing="controller",
            availability="disconnected",
            availability_reason="Ollama is starting.",
        )
        worker = AgentConfig(
            id="worker",
            role="local-agent",
            provider="ollama",
            runner="ollama",
            model="worker-model",
        )

        payload = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[frontier, router, controller]),
            global_skill_roots=[],
            ollama_probe=self.healthy_ollama_probe(
                "frontier-model",
                "router-model",
                "controller-model",
            ),
        ).inspect().to_dict()

        self.assertEqual(payload["default_agent_id"], "router")
        self.assertIs(
            AgentRegistry(agents=[frontier, router, controller]).controller_agent(),
            router,
        )
        self.assertEqual(
            payload["agents"],
            [
                {
                    "id": "frontier",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "frontier-model",
                    "routing": "",
                    "availability": "available",
                    "availability_reason": "",
                    "assignable": False,
                    "delegate_only": False,
                    "requires_approval": False,
                },
                {
                    "id": "router",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "router-model",
                    "routing": "router",
                    "availability": "available",
                    "availability_reason": "",
                    "assignable": False,
                    "delegate_only": False,
                    "requires_approval": False,
                },
                {
                    "id": "controller",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "controller-model",
                    "routing": "controller",
                    "availability": "disconnected",
                    "availability_reason": "Ollama is starting.",
                    "assignable": False,
                    "delegate_only": False,
                    "requires_approval": False,
                },
            ],
        )

        for agents, expected_default in (
            ([frontier, router], "router"),
            ([frontier], "frontier"),
            ([worker], ""),
            ([], ""),
        ):
            with self.subTest(expected_default=expected_default):
                fallback = CapabilityCatalogService(
                    workspace_root=self.workspace,
                    agent_registry=AgentRegistry(agents=agents),
                    global_skill_roots=[],
                    ollama_probe=self.healthy_ollama_probe(
                        "frontier-model",
                        "router-model",
                        "worker-model",
                    ),
                ).inspect()
                self.assertEqual(fallback.default_agent_id, expected_default)

    def test_builtin_slash_commands_have_stable_json_ready_metadata(self) -> None:
        payload = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(agents=[]),
            global_skill_roots=[],
        ).inspect().to_dict()

        self.assertEqual(
            payload["commands"],
            [
                {
                    "name": "/help",
                    "usage": "/help",
                    "description": "Show available commands and invocation help.",
                    "category": "discovery",
                },
                {
                    "name": "/skills",
                    "usage": "/skills [query]",
                    "description": "Browse installed skills, optionally filtered by a query.",
                    "category": "discovery",
                },
                {
                    "name": "/use",
                    "usage": "/use <skill> [request]",
                    "description": "Invoke an installed skill for the optional request.",
                    "category": "agent",
                },
                {
                    "name": "/run",
                    "usage": "/run <command>",
                    "description": "Run a governed shell command in the active workspace.",
                    "category": "execution",
                },
                {
                    "name": "/task",
                    "usage": "/task <request>",
                    "description": "Start or steer coding work with the selected controller.",
                    "category": "agent",
                },
                {
                    "name": "/status",
                    "usage": "/status",
                    "description": (
                        "Show controller, subagent, ready-work, and snapshot-storage "
                        "status."
                    ),
                    "category": "monitoring",
                },
                {
                    "name": "/storage",
                    "usage": "/storage",
                    "description": (
                        "Inspect Snapshot Payload usage, expiry, reclamation, and "
                        "blockers."
                    ),
                    "category": "monitoring",
                },
            ],
        )

    def test_catalog_marks_only_valid_manual_assignment_agents_assignable(self) -> None:
        worker = AgentConfig(
            id="worker",
            role="local-agent",
            provider="ollama",
            runner="ollama",
            model="worker-model",
        )
        gated_worker = AgentConfig(
            id="gated-worker",
            role="local-agent",
            provider="local",
            runner="fake",
            requires_approval=True,
        )
        cloud_worker = AgentConfig(
            id="cloud-worker",
            role="local-agent",
            provider="ollama",
            runner="fake",
            model="remote:CLOUD",
        )
        remote_provider_worker = AgentConfig(
            id="remote-provider-worker",
            role="local-agent",
            provider="remote",
            runner="fake",
            model="remote-provider-worker-model",
        )
        remote_runner_worker = AgentConfig(
            id="remote-runner-worker",
            role="local-agent",
            provider="local",
            runner="remote-api",
            model="remote-runner-worker-model",
        )
        delegate = AgentConfig(
            id="delegate",
            role="local-agent",
            provider="ollama",
            runner="ollama",
            model="delegate-model",
            delegate_only=True,
            requires_approval=True,
        )
        controller = AgentConfig(
            id="controller",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="controller-model",
            routing="controller",
        )
        frontier_routed_worker = AgentConfig(
            id="frontier-routed-worker",
            role="local-agent",
            provider="ollama",
            runner="ollama",
            model="frontier-routed-worker-model",
            routing="frontier",
        )
        unrelated_local_agent = AgentConfig(
            id="unrelated-local-agent",
            role="observer",
            provider="local",
            runner="fake",
        )
        unavailable_worker = AgentConfig(
            id="unavailable-worker",
            role="local-agent",
            provider="local",
            runner="fake",
            availability="unavailable",
            availability_reason="worker is disabled",
        )

        agents = CapabilityCatalogService(
            workspace_root=self.workspace,
            agent_registry=AgentRegistry(
                agents=[
                    worker,
                    gated_worker,
                    cloud_worker,
                    remote_provider_worker,
                    remote_runner_worker,
                    delegate,
                    controller,
                    frontier_routed_worker,
                    unrelated_local_agent,
                    unavailable_worker,
                ]
            ),
            global_skill_roots=[],
            ollama_probe=self.healthy_ollama_probe(
                "worker-model",
                "delegate-model",
                "controller-model",
                "frontier-routed-worker-model",
            ),
        ).inspect().to_dict()["agents"]

        self.assertEqual(
            [(agent["id"], agent["assignable"]) for agent in agents],
            [
                ("worker", True),
                ("gated-worker", False),
                ("cloud-worker", False),
                ("remote-provider-worker", False),
                ("remote-runner-worker", False),
                ("delegate", False),
                ("controller", False),
                ("frontier-routed-worker", False),
                ("unrelated-local-agent", False),
                ("unavailable-worker", True),
            ],
        )
        unavailable_projection = next(
            agent for agent in agents if agent["id"] == "unavailable-worker"
        )
        self.assertEqual(unavailable_projection["availability"], "unavailable")
        delegate_projection = next(
            agent for agent in agents if agent["id"] == "delegate"
        )
        self.assertTrue(delegate_projection["delegate_only"])
        self.assertTrue(delegate_projection["requires_approval"])

    def test_registry_excludes_gated_and_cloud_controllers(self) -> None:
        cloud = AgentConfig(
            id="cloud-controller",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="qwen:cloud",
            routing="controller",
        )
        gated = AgentConfig(
            id="gated-controller",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="qwen:14b",
            routing="controller",
            requires_approval=True,
        )
        local = AgentConfig(
            id="local-controller",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="qwen:14b",
            routing="controller",
        )

        self.assertIs(
            AgentRegistry(agents=[cloud, gated, local]).controller_agent(),
            local,
        )
        self.assertIsNone(AgentRegistry(agents=[cloud, gated]).controller_agent())

    def test_registry_controller_selection_uses_local_available_routing_boundaries(self) -> None:
        misrouted_frontier = AgentConfig(
            id="misrouted-frontier",
            role="frontier",
            provider="local",
            runner="fake",
            routing="worker",
        )
        unavailable = AgentConfig(
            id="unavailable-controller",
            role="frontier",
            provider="local",
            runner="fake",
            routing="controller",
            availability="unavailable",
        )
        uppercase_cloud = AgentConfig(
            id="uppercase-cloud-router",
            role="frontier",
            provider="ollama",
            runner="ollama",
            model="remote:CLOUD",
            routing="router",
        )
        fallback = AgentConfig(
            id="unrouted-frontier",
            role="frontier",
            provider="local",
            runner="fake",
        )

        self.assertTrue(is_cloud_model("remote:CLOUD"))
        self.assertFalse(is_cloud_model("local:14b"))
        self.assertIs(
            AgentRegistry(
                agents=[
                    misrouted_frontier,
                    unavailable,
                    uppercase_cloud,
                    fallback,
                ]
            ).controller_agent(),
            fallback,
        )
        self.assertIsNone(
            AgentRegistry(
                agents=[misrouted_frontier, unavailable, uppercase_cloud]
            ).controller_agent()
        )

    def test_agent_capabilities_cli_prints_the_versioned_catalog_as_json(self) -> None:
        local_skill = self.write_skill(
            self.workspace / ".agents" / "skills",
            "local",
            name="local-skill",
            description="Workspace-local capability.",
        )
        configured_root = self.root / "configured-skills"
        self.write_skill(
            configured_root,
            "configured",
            name="configured-skill",
            description="Configured capability.",
        )
        global_root = self.root / "global-skills"
        self.write_skill(
            global_root,
            "global",
            name="global-skill",
            description="Global capability.",
        )
        agent_config = self.root / "agents.json"
        agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "qwen-controller",
                            "role": "frontier",
                            "provider": "ollama",
                            "runner": "ollama",
                            "model": "qwen:latest",
                            "routing": "controller",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        output = io.StringIO()

        with patch(
            "albert_mvp.capabilities.OllamaHealthProbe.__call__",
            return_value=OllamaHealthSnapshot(
                reachable=True,
                models=frozenset({"qwen:latest"}),
            ),
        ), redirect_stdout(output):
            exit_code = main(
                [
                    "agent-capabilities",
                    "--target-repo",
                    str(self.workspace),
                    "--tracker-dir",
                    str(self.root / "unused-tracker"),
                    "--runtime-root",
                    str(self.root / "unused-runtime"),
                    "--agent-config",
                    str(agent_config),
                    "--skill-root",
                    str(configured_root),
                    "--global-skill-root",
                    str(global_root),
                ]
            )

        payload = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(
            list(payload),
            ["agents", "commands", "default_agent_id", "schema_version", "skills"],
        )
        self.assertEqual(payload["default_agent_id"], "qwen-controller")
        self.assertEqual(
            [skill["name"] for skill in payload["skills"]],
            ["configured-skill", "global-skill", "local-skill"],
        )
        self.assertEqual(payload["skills"][2]["source"], str(local_skill.resolve()))
        self.assertEqual(
            [command["name"] for command in payload["commands"]],
            [
                "/help",
                "/skills",
                "/use",
                "/run",
                "/task",
                "/status",
                "/storage",
            ],
        )
        self.assertEqual(payload["agents"][0]["model"], "qwen:latest")
        self.assertEqual(payload["agents"][0]["availability"], "available")

    def test_default_global_roots_include_personal_codex_and_plugin_skills(self) -> None:
        home = self.root / "home"
        codex_home = self.root / "codex-home"
        self.write_skill(
            home / ".agents" / "skills",
            "personal",
            name="personal-skill",
            description="Personal capability.",
        )
        self.write_skill(
            codex_home / "skills",
            "system",
            name="system-skill",
            description="Codex capability.",
        )
        self.write_skill(
            codex_home / "plugins" / "cache" / "example" / "1.0" / "skills",
            "plugin",
            name="plugin-skill",
            description="Plugin capability.",
        )

        with patch.dict(
            os.environ,
            {"HOME": str(home), "CODEX_HOME": str(codex_home)},
        ):
            payload = CapabilityCatalogService(
                workspace_root=self.workspace,
                agent_registry=AgentRegistry(agents=[]),
            ).inspect().to_dict()

        self.assertEqual(
            [skill["name"] for skill in payload["skills"]],
            ["personal-skill", "plugin-skill", "system-skill"],
        )

    def test_agent_capabilities_cli_uses_workspace_agent_registry_by_default(self) -> None:
        agent_config = self.workspace / ".albert" / "agents.json"
        agent_config.parent.mkdir()
        agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "workspace-router",
                            "role": "frontier",
                            "provider": "ollama",
                            "runner": "ollama",
                            "model": "workspace-model",
                            "routing": "router",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        output = io.StringIO()

        with patch.dict(
            os.environ,
            {"HOME": str(self.root / "empty-home"), "CODEX_HOME": str(self.root / "empty-codex")},
        ), patch(
            "albert_mvp.capabilities.OllamaHealthProbe.__call__",
            return_value=OllamaHealthSnapshot(
                reachable=True,
                models=frozenset({"workspace-model"}),
            ),
        ), redirect_stdout(output):
            exit_code = main(
                [
                    "agent-capabilities",
                    "--target-repo",
                    str(self.workspace),
                ]
            )

        payload = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["default_agent_id"], "workspace-router")
        self.assertEqual(payload["agents"][0]["model"], "workspace-model")


if __name__ == "__main__":
    unittest.main()
