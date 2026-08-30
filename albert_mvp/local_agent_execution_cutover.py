"""Effect-specific provider selection for Local Agent host execution."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Mapping

from .execution import (
    ExecutionContractError,
    ExecutionExecutor,
    ExecutionProvider,
    PythonExecutionProvider,
)
from .execution_cutover import (
    RUST_CANDIDATE_ENABLED,
    RUST_EXECUTION_PROVIDER,
    RUST_EXECUTION_PROVIDER_SHA256,
    RustExecutionProvider,
)


RUST_LOCAL_AGENT_ENABLED = "ALFREDO_RUST_LOCAL_AGENT_ENABLED"


def local_agent_execution_provider_from_environment(
    environment: Mapping[str, str] | None = None,
    *,
    python_executor: ExecutionExecutor | None = None,
) -> ExecutionProvider:
    """Select one provider before Local Agent authorization is journaled."""

    values = os.environ if environment is None else environment
    enabled = values.get(RUST_LOCAL_AGENT_ENABLED, "0")
    if enabled == "0":
        return PythonExecutionProvider(executor=python_executor)
    if enabled != "1":
        raise ExecutionContractError(f"{RUST_LOCAL_AGENT_ENABLED} must be 0 or 1")

    candidate_enabled = values.get(RUST_CANDIDATE_ENABLED, "0")
    if candidate_enabled == "0":
        return PythonExecutionProvider(executor=python_executor)
    if candidate_enabled != "1":
        raise ExecutionContractError(f"{RUST_CANDIDATE_ENABLED} must be 0 or 1")

    rust_provider = RustExecutionProvider(
        Path(values.get(RUST_EXECUTION_PROVIDER, "")),
        values.get(RUST_EXECUTION_PROVIDER_SHA256, ""),
    )
    try:
        rust_provider.verify_available()
    except ExecutionContractError:
        # Selection happens before ExecutionCoordinator validates or claims the
        # request, proving there is no canonical write, effect, or open receipt.
        return PythonExecutionProvider(executor=python_executor)
    return rust_provider


__all__ = [
    "RUST_LOCAL_AGENT_ENABLED",
    "local_agent_execution_provider_from_environment",
]
