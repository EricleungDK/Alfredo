import {
  copyFileSync,
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  rmSync,
} from "node:fs";
import { basename, dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");
const stagingRoot = resolve(projectRoot, "bundled-backend");
const sourcePackage = resolve(repositoryRoot, "albert_mvp");
const sourceAgentConfig = resolve(repositoryRoot, ".albert", "agents.json");

function assertSafeStagingRoot() {
  if (dirname(stagingRoot) !== projectRoot || basename(stagingRoot) !== "bundled-backend") {
    throw new Error(`Refusing unsafe Alfredo backend staging path: ${stagingRoot}`);
  }
}

function assertRegularSource(path, kind) {
  if (!existsSync(path)) {
    throw new Error(`Cannot stage Alfredo ${kind}; source is missing: ${path}`);
  }
  const source = lstatSync(path);
  const expectedKind = kind === "backend" ? "directory" : "file";
  const hasExpectedKind = kind === "backend" ? source.isDirectory() : source.isFile();
  if (source.isSymbolicLink() || !hasExpectedKind) {
    throw new Error(`Cannot stage Alfredo ${kind}; source is not a regular ${expectedKind}: ${path}`);
  }
}

function clean() {
  assertSafeStagingRoot();
  rmSync(stagingRoot, { recursive: true, force: true });
}

function includeBackendSource(source) {
  const sourceRelative = relative(sourcePackage, source);
  if (!sourceRelative) return true;
  const parts = sourceRelative.split(sep);
  if (parts.includes("__pycache__") || source.endsWith(".pyc")) return false;
  const entry = lstatSync(source);
  return !entry.isSymbolicLink() && (entry.isDirectory() || entry.isFile());
}

function stage() {
  clean();
  try {
    assertRegularSource(sourcePackage, "backend");
    assertRegularSource(sourceAgentConfig, "agent config");
    cpSync(sourcePackage, resolve(stagingRoot, "albert_mvp"), {
      recursive: true,
      dereference: false,
      filter: includeBackendSource,
    });
    mkdirSync(resolve(stagingRoot, ".albert"), { recursive: true });
    copyFileSync(sourceAgentConfig, resolve(stagingRoot, ".albert", "agents.json"));
  } catch (error) {
    clean();
    throw error;
  }
}

const action = process.argv[2];
if (action === "stage") {
  stage();
} else if (action === "clean") {
  clean();
} else {
  throw new Error('Expected backend staging action "stage" or "clean".');
}
