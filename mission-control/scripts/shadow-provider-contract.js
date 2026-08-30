import { createHash } from "node:crypto";

const DIGEST_PATTERN = /^[a-f0-9]{64}$/;

const EXPECTED_SHADOW_COHORTS = Object.freeze([
  Object.freeze({ request_id: "packaged-shadow-completed", status: "completed" }),
  Object.freeze({ request_id: "packaged-shadow-failed", status: "failed" }),
  Object.freeze({ request_id: "packaged-shadow-timeout-cleanup", status: "timed-out" }),
  Object.freeze({ request_id: "packaged-shadow-output-limit", status: "output-limit" }),
  Object.freeze({ request_id: "packaged-shadow-cancellation", status: "cancelled" }),
  Object.freeze({ request_id: "packaged-shadow-replay", status: "provider-free-replay" }),
  Object.freeze({ request_id: "packaged-shadow-crash-cut", status: "outcome-unknown" }),
  Object.freeze({
    request_id: "packaged-shadow-resource-validation",
    status: "contract-failure",
  }),
  Object.freeze({
    request_id: "packaged-shadow-sandbox-validation",
    status: "contract-failure",
  }),
  Object.freeze({
    request_id: "packaged-shadow-state-version",
    status: "contract-failure",
  }),
]);

function validateShadowProviderParityEvidence(
  evidence,
  { providerSha256, canonicalStoreRoots },
) {
  if (
    !evidence ||
    evidence.status !== "pass" ||
    evidence.provider_sha256 !== providerSha256 ||
    evidence.receipt_status !== "completed" ||
    evidence.store_unchanged !== true ||
    !Array.isArray(evidence.canonical_store_roots) ||
    JSON.stringify(evidence.canonical_store_roots) !== JSON.stringify(canonicalStoreRoots) ||
    !Array.isArray(evidence.cohorts) ||
    evidence.cohorts.length !== EXPECTED_SHADOW_COHORTS.length
  ) {
    throw new Error("Packaged shadow evidence does not match the exact contract.");
  }

  for (const [index, expected] of EXPECTED_SHADOW_COHORTS.entries()) {
    const cohort = evidence.cohorts[index];
    if (
      !cohort ||
      cohort.request_id !== expected.request_id ||
      cohort.status !== expected.status ||
      typeof cohort.request_sha256 !== "string" ||
      !DIGEST_PATTERN.test(cohort.request_sha256) ||
      cohort.store_unchanged !== true
    ) {
      throw new Error("Packaged shadow evidence does not match the exact contract.");
    }
    if (
      expected.request_id === "packaged-shadow-timeout-cleanup" &&
      cohort.cleanup_verified !== true
    ) {
      throw new Error("Packaged shadow evidence lacks the descendant cleanup proof.");
    }
    if (
      expected.request_id === "packaged-shadow-crash-cut" &&
      cohort.normalized_parity !== true
    ) {
      throw new Error("Packaged shadow evidence lacks full crash normalized parity.");
    }
  }

  const suiteDigest = createHash("sha256")
    .update(
      evidence.cohorts.map((cohort) => cohort.request_sha256).join("\n"),
      "ascii",
    )
    .digest("hex");
  if (evidence.request_sha256 !== suiteDigest) {
    throw new Error("Packaged shadow evidence suite digest is invalid.");
  }
  return evidence;
}

export { EXPECTED_SHADOW_COHORTS, validateShadowProviderParityEvidence };
