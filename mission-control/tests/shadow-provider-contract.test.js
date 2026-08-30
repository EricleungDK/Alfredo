import { createHash } from "node:crypto";

import { expect, test } from "vitest";

import {
  EXPECTED_SHADOW_COHORTS,
  validateShadowProviderParityEvidence,
} from "../scripts/shadow-provider-contract.js";

const digest = "a".repeat(64);

function passingEvidence() {
  const cohorts = EXPECTED_SHADOW_COHORTS.map(({ request_id, status, ...contract }) => ({
    request_id,
    request_sha256: digest,
    status,
    ...contract,
    store_unchanged: true,
    ...(request_id === "packaged-shadow-timeout-cleanup"
      ? { cleanup_verified: true }
      : {}),
    ...(request_id === "packaged-shadow-crash-cut"
      ? { normalized_parity: true }
      : {}),
  }));
  return {
    status: "pass",
    provider_sha256: digest,
    receipt_status: "completed",
    store_unchanged: true,
    canonical_store_roots: ["/workspace/shadow-release-sentinel", "/runtime"],
    python_fallback: {
      selection_boundary: "pre-effect",
      shell: "python",
      local_agent: "python",
    },
    cohorts,
    request_sha256: createHash("sha256")
      .update(cohorts.map((cohort) => cohort.request_sha256).join("\n"), "ascii")
      .digest("hex"),
  };
}

test("packaged shadow evidence requires the exact thirteen-cohort contract", () => {
  const evidence = passingEvidence();
  expect(() =>
    validateShadowProviderParityEvidence(evidence, {
      providerSha256: digest,
      canonicalStoreRoots: ["/workspace/shadow-release-sentinel", "/runtime"],
    }),
  ).not.toThrow();

  expect(() =>
    validateShadowProviderParityEvidence(
      { ...evidence, cohorts: evidence.cohorts.slice(0, -1) },
      {
        providerSha256: digest,
        canonicalStoreRoots: ["/workspace/shadow-release-sentinel", "/runtime"],
      },
    ),
  ).toThrow(/exact contract/);

  const wrongStatus = structuredClone(evidence);
  wrongStatus.cohorts[4].status = "completed";
  expect(() =>
    validateShadowProviderParityEvidence(wrongStatus, {
      providerSha256: digest,
      canonicalStoreRoots: ["/workspace/shadow-release-sentinel", "/runtime"],
    }),
  ).toThrow(/exact contract/);

  const missingCleanup = structuredClone(evidence);
  delete missingCleanup.cohorts.find(
    ({ request_id }) => request_id === "packaged-shadow-timeout-cleanup",
  ).cleanup_verified;
  expect(() =>
    validateShadowProviderParityEvidence(missingCleanup, {
      providerSha256: digest,
      canonicalStoreRoots: ["/workspace/shadow-release-sentinel", "/runtime"],
    }),
  ).toThrow(/cleanup proof/);
});
