//! Explicit diagnostic experiments. Reports never approve work or promote a profile.
use crate::{
    inference_admission::Class,
    inference_profile::{ContextProfile, RequestOutcome, RequestRecord, RequestRecorder},
    inference_runtime::{self, Observation},
    provider::Ollama,
    qualification_runner::{self, FixtureDefinition, Scenario, ScenarioResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUESTS: usize = 128;
const DEADLINE_SECONDS: u64 = 1800;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| e.to_string())
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(500)
        .collect()
}
fn bounded_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseKey {
    pub repetition: u8,
    pub profile: ContextProfile,
    pub scenario: Scenario,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub endpoint_origin: String,
    pub model: String,
    pub executable_sha256: String,
    /// A version/catalog observation is not a verified upstream binary/configuration pin.
    pub upstream_binary_pin_verified: bool,
    pub repetitions: u8,
    pub capacity: usize,
    pub max_requests: usize,
    pub deadline_seconds: u64,
    pub structured_thinking: Option<bool>,
    pub fixtures: Vec<FixtureDefinition>,
    pub schedule: Vec<CaseKey>,
    pub initial_runtime: Option<Observation>,
    pub initial_error: Option<String>,
    pub artifact_directory: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Pending,
    Running,
    Finished,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub key: CaseKey,
    pub phase: Phase,
    pub before: Option<Observation>,
    pub after: Option<Observation>,
    pub result: Option<ScenarioResult>,
    pub error: Option<String>,
    pub inspection_error: Option<String>,
    pub requests: Vec<RequestRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub manifest: Manifest,
    pub manifest_sha256: String,
    pub cases: Vec<Case>,
    pub finished: bool,
    pub stop_reason: Option<String>,
    pub sha256: String,
}
fn schedule(repetitions: u8) -> Vec<CaseKey> {
    let mut schedule = vec![];
    for repetition in 1..=repetitions {
        for scenario in Scenario::ALL {
            // Alternate pair order; do not label first runs cold without observation.
            let profiles = if repetition % 2 == 1 {
                [ContextProfile::Baseline, ContextProfile::ContextCandidate]
            } else {
                [ContextProfile::ContextCandidate, ContextProfile::Baseline]
            };
            for profile in profiles {
                schedule.push(CaseKey {
                    repetition,
                    profile,
                    scenario,
                });
            }
        }
    }
    schedule
}
impl Manifest {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || !(1..=3).contains(&self.repetitions)
            || self.capacity != 1
            || self.max_requests != MAX_REQUESTS
            || self.deadline_seconds != DEADLINE_SECONDS
            || self.upstream_binary_pin_verified
            || !valid_digest(&self.executable_sha256)
            || !bounded_text(&self.model, 200)
            || self.schedule != schedule(self.repetitions)
            || self.fixtures.len() != 4
            || !self.artifact_directory.is_absolute()
            || self.artifact_directory.to_string_lossy().len() > 4096
            || self.initial_runtime.is_some() == self.initial_error.is_some()
            || self
                .initial_error
                .as_ref()
                .is_some_and(|e| !bounded_text(e, 2048))
        {
            return Err("Invalid qualification manifest".into());
        }
        crate::inference_admission::Coordinator::new(self.endpoint_origin.clone(), 1)?;
        for (fixture, scenario) in self.fixtures.iter().zip(Scenario::ALL) {
            fixture.validate()?;
            if fixture.scenario != scenario
                || fixture.digest() != qualification_runner::fixture_definition(scenario).digest()
            {
                return Err("Qualification fixture definition changed".into());
            }
        }
        if let Some(observation) = &self.initial_runtime {
            self.validate_runtime(observation)?;
        }
        Ok(())
    }
    fn validate_runtime(&self, observation: &Observation) -> Result<()> {
        observation.validate()?;
        if observation.endpoint_origin != self.endpoint_origin
            || observation.selected_model != self.model
        {
            return Err("Qualification runtime belongs to a different endpoint or model".into());
        }
        Ok(())
    }
}
impl Report {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.cases.len() != self.manifest.schedule.len()
            || self.manifest_sha256 != digest(&json(&self.manifest)?)
            || self
                .stop_reason
                .as_ref()
                .is_some_and(|e| !bounded_text(e, 2048))
        {
            return Err("Qualification report identity is invalid".into());
        }
        self.manifest.validate()?;
        let mut next_request = 1;
        let mut unfinished = false;
        for (index, (case, key)) in self.cases.iter().zip(&self.manifest.schedule).enumerate() {
            if &case.key != key {
                return Err("Qualification case schedule changed".into());
            }
            if unfinished && case.phase != Phase::Pending {
                return Err("Qualification cases are out of order".into());
            }
            unfinished |= case.phase != Phase::Finished;
            if case
                .error
                .iter()
                .chain(case.inspection_error.iter())
                .any(|e| !bounded_text(e, 2048))
            {
                return Err("Invalid qualification error".into());
            }
            for observation in case.before.iter().chain(case.after.iter()) {
                self.manifest.validate_runtime(observation)?;
            }
            match case.phase {
                Phase::Pending
                    if case.before.is_some()
                        || case.after.is_some()
                        || case.result.is_some()
                        || case.error.is_some()
                        || case.inspection_error.is_some()
                        || !case.requests.is_empty() =>
                {
                    return Err("Pending case contains fabricated observations".into())
                }
                Phase::Running
                    if case.after.is_some()
                        || case.result.is_some()
                        || case.error.is_some()
                        || case.inspection_error.is_some()
                        || !case.requests.is_empty() =>
                {
                    return Err("Running checkpoint contains an unsupported outcome".into())
                }
                Phase::Finished if case.result.is_some() == case.error.is_some() => {
                    return Err("Finished case must have one recorded result".into())
                }
                _ => {}
            }
            if let Some(result) = &case.result {
                result.validate()?;
                validate_case_requests(result, &case.requests)?;
                if result.scenario != key.scenario
                    || result.fixture_digest
                        != qualification_runner::fixture_definition(key.scenario).digest()
                    || result.artifact_directory
                        != self
                            .manifest
                            .artifact_directory
                            .join(format!("case-{:02}", index + 1))
                    || (result.accepted() && case.requests.is_empty())
                {
                    return Err("Qualification result has different fixture or run identity".into());
                }
            }
            for request in &case.requests {
                request.validate()?;
                if request.sequence != next_request
                    || !request.matches_binding(
                        key.profile,
                        &self.manifest.endpoint_origin,
                        &self.manifest.model,
                        1,
                    )
                    || request.profile.idle_timeout_ms != 60_000
                    || (request.profile.format_sha256.is_some()
                        && request.profile.think != self.manifest.structured_thinking)
                {
                    return Err("Qualification request identity or sequence changed".into());
                }
                if request.outcome == RequestOutcome::InFlight {
                    return Err("Finished case retains an in-flight request".into());
                }
                next_request += 1;
            }
        }
        if next_request > MAX_REQUESTS + 1
            || (self.finished && unfinished)
            || (self.finished && self.stop_reason.is_some())
        {
            return Err("Qualification completion or request bound is invalid".into());
        }
        let mut unsigned = self.clone();
        unsigned.sha256.clear();
        if self.sha256 != digest(&json(&unsigned)?) {
            return Err("Qualification report checksum mismatch".into());
        }
        Ok(())
    }
    fn seal(&mut self) -> Result<()> {
        self.sha256.clear();
        self.sha256 = digest(&json(self)?);
        self.validate()
    }
    pub fn case_issues(&self, case: &Case) -> Vec<String> {
        let mut issues = vec![];
        if case.phase != Phase::Finished {
            issues.push("Scenario did not finish".into());
        }
        for error in case.error.iter().chain(case.inspection_error.iter()) {
            issues.push(error.clone());
        }
        if !case.result.as_ref().is_some_and(ScenarioResult::accepted) {
            issues.push("No canonical accepted fixture outcome".into());
        }
        match (&self.manifest.initial_runtime, &case.before, &case.after) {
            (Some(initial), Some(before), Some(after)) => {
                issues.extend(initial.drift_reasons(before));
                issues.extend(before.drift_reasons(after));
                issues.extend(after.missing_reasons());
            }
            _ => issues.push("Runtime inspection is incomplete".into()),
        }
        if case.requests.is_empty() {
            issues.push("No recorded generation request".into());
        }
        for request in &case.requests {
            if request.outcome != RequestOutcome::Completed {
                issues.push("A generation request did not complete".into());
            }
            if !request.metrics.as_ref().is_some_and(|m| {
                m.total_duration.is_some()
                    && m.load_duration.is_some()
                    && m.prompt_eval_duration.is_some()
                    && m.eval_duration.is_some()
                    && m.prompt_eval_count.is_some()
                    && m.eval_count.is_some()
            }) {
                issues.push("Server timing/usage is incomplete".into());
            }
        }
        for request in &case.requests {
            if let Some(error) = &request.runtime_error {
                issues.push(error.clone());
            }
            match &request.runtime_after {
                Some(observation) => {
                    issues.extend(observation.request_reasons(
                        &self.manifest.endpoint_origin,
                        &self.manifest.model,
                        request.profile.num_ctx,
                    ));
                    if let Some(initial) = &self.manifest.initial_runtime {
                        issues.extend(initial.drift_reasons(observation));
                    }
                }
                None => issues.push("Per-request runtime context was not observed".into()),
            }
        }
        issues.sort();
        issues.dedup();
        issues
    }
    pub fn summary(&self) -> String {
        let attempted = self
            .cases
            .iter()
            .filter(|c| c.phase != Phase::Pending)
            .count();
        let accepted = self
            .cases
            .iter()
            .filter(|c| c.result.as_ref().is_some_and(ScenarioResult::accepted))
            .count();
        let observed = self
            .cases
            .iter()
            .filter(|c| self.case_issues(c).is_empty())
            .count();
        format!("Diagnostic cohort {}: {attempted}/{} attempted, {accepted} canonical accepted, {observed} with complete model/runtime/timing observations. Upstream binary pin and exact token headroom unverified; no promotion or general speed claim.", if self.finished { "finished" } else { "incomplete" }, self.cases.len())
    }
}

fn validate_case_requests(result: &ScenarioResult, requests: &[RequestRecord]) -> Result<()> {
    let expected = match result.scenario {
        Scenario::SmallEdit | Scenario::RequiredSource => 2,
        Scenario::Repair | Scenario::QueuedForeground => 3,
    };
    if requests.len() > result.generation_attempts as usize
        || requests.len() > expected
        || (result.accepted()
            && (requests.len() != expected || result.generation_attempts as usize != expected))
    {
        return Err("Qualification requests do not cover the governed generation attempts".into());
    }
    for (index, request) in requests.iter().enumerate() {
        let discussion = index == 2 && result.scenario == Scenario::QueuedForeground;
        let class = if index == 0 || discussion {
            Class::Foreground
        } else {
            Class::Background
        };
        if request.session != 0
            || request.attempt != u64::from(index != 0)
            || request.profile.class != class
            || request.profile.format_sha256.is_some() == discussion
        {
            return Err("Qualification request differs from its governed phase".into());
        }
        if discussion {
            let fixture = qualification_runner::fixture_definition(result.scenario);
            if request.messages.len() != 1
                || request.messages[0].role != "user"
                || Some(&request.messages[0].content_sha256)
                    != fixture.foreground_prompt_sha256.as_ref()
            {
                return Err(
                    "Qualification foreground request differs from its fixed prompt".into(),
                );
            }
        }
    }
    if let Some(sequence) = result
        .foreground
        .as_ref()
        .and_then(|f| f.background_request)
    {
        if requests.get(1).is_none_or(|request| {
            request.sequence != sequence || request.profile.class != Class::Background
        }) {
            return Err(
                "Qualification queue observation belongs to a different worker request".into(),
            );
        }
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Report> {
    let mut bytes = vec![];
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Report must be a regular file".into());
    }
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Qualification report exceeds 4 MiB".into());
    }
    let report: Report =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid qualification report: {e}"))?;
    report.validate()?;
    Ok(report)
}
struct Output {
    path: PathBuf,
    artifacts: PathBuf,
    created: bool,
    last_identity: Option<(u64, u64)>,
    last_sha256: Option<String>,
}
fn identity(file: &File) -> Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err("Qualification artifacts require a supported Unix host".into())
    }
}
impl Output {
    fn new(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|e| format!("Report parent unavailable: {e}"))?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| bounded_text(s, 160))
            .ok_or("Invalid report filename")?;
        let path = parent.join(name);
        if fs::symlink_metadata(&path).is_ok() {
            return Err("Qualification report already exists; choose a new path. Reports never resume inference".into());
        }
        let artifacts = parent.join(format!("{name}.artifacts"));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&artifacts)
            .map_err(|e| format!("Cannot reserve fresh qualification artifacts: {e}"))?;
        Ok(Self {
            path,
            artifacts,
            created: false,
            last_identity: None,
            last_sha256: None,
        })
    }
    fn save(&mut self, report: &mut Report) -> Result<()> {
        report.seal()?;
        if self.created {
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options
                .open(&self.path)
                .map_err(|e| format!("Owned report is unavailable; preserved: {e}"))?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file()
                || Some(identity(&file)?) != self.last_identity
            {
                return Err("Qualification report was replaced; replacement preserved".into());
            }
            let mut current = Vec::new();
            file.take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut current)
                .map_err(|e| e.to_string())?;
            if current.len() > MAX_BYTES || Some(digest(&current)) != self.last_sha256 {
                return Err("Qualification report was edited; changed bytes preserved".into());
            }
        }
        let bytes = json(report)?;
        if bytes.len() > MAX_BYTES {
            return Err("Qualification report exceeds 4 MiB".into());
        }
        let temporary = self.artifacts.join("report.pending");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        let next_identity = identity(&file)?;
        if self.created {
            fs::rename(&temporary, &self.path).map_err(|e| e.to_string())?;
        } else {
            fs::hard_link(&temporary, &self.path)
                .map_err(|e| format!("Cannot publish new report without overwrite: {e}"))?;
            fs::remove_file(&temporary).map_err(|e| e.to_string())?;
            self.created = true;
        }
        self.last_identity = Some(next_identity);
        self.last_sha256 = Some(digest(&bytes));
        File::open(self.path.parent().unwrap())
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("Report publication unconfirmed: {e}"))
    }
}

/// Standalone, explicitly requested experiment. The fixed one-slot cohort does not
/// change production provider defaults; existing endpoint capacity conflicts refuse.
pub async fn run(
    path: &Path,
    endpoint: &str,
    model: &str,
    repetitions: u8,
    thinking: Option<bool>,
    cancel: Arc<AtomicBool>,
) -> Result<Report> {
    let started = Instant::now();
    if !(1..=3).contains(&repetitions) {
        return Err("Qualification repetitions must be 1–3".into());
    }
    let provider = Ollama::new(endpoint, Duration::from_secs(60))?
        .with_parallelism(1)?
        .with_structured_thinking(thinking);
    let origin = reqwest::Url::parse(endpoint)
        .map_err(|e| e.to_string())?
        .origin()
        .ascii_serialization();
    let initial = inference_runtime::inspect(&provider, model).await;
    let executable_sha256 = digest(
        &fs::read(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    );
    let mut output = Output::new(path)?;
    let manifest = Manifest {
        schema_version: 1,
        endpoint_origin: origin,
        model: model.into(),
        executable_sha256,
        upstream_binary_pin_verified: false,
        repetitions,
        capacity: 1,
        max_requests: MAX_REQUESTS,
        deadline_seconds: DEADLINE_SECONDS,
        structured_thinking: thinking,
        fixtures: Scenario::ALL
            .into_iter()
            .map(qualification_runner::fixture_definition)
            .collect(),
        schedule: schedule(repetitions),
        initial_runtime: initial.as_ref().ok().cloned(),
        initial_error: initial.err().map(|e| clean(&e)),
        artifact_directory: output.artifacts.clone(),
    };
    let cases = manifest
        .schedule
        .iter()
        .map(|key| Case {
            key: key.clone(),
            phase: Phase::Pending,
            before: None,
            after: None,
            result: None,
            error: None,
            inspection_error: None,
            requests: vec![],
        })
        .collect();
    let mut report = Report {
        schema_version: 1,
        manifest_sha256: digest(&json(&manifest)?),
        manifest,
        cases,
        finished: false,
        stop_reason: None,
        sha256: String::new(),
    };
    output.save(&mut report)?;
    let recorder = RequestRecorder::with_limit(MAX_REQUESTS)?;
    let provider = provider.with_request_recorder(recorder.clone());
    let mut cursor = 0;
    for index in 0..report.cases.len() {
        if cancel.load(Ordering::SeqCst)
            || started.elapsed() >= Duration::from_secs(DEADLINE_SECONDS)
        {
            report.stop_reason = Some(
                "Cohort cancelled or deadline reached; remaining cases were not started".into(),
            );
            break;
        }
        if report.manifest.initial_runtime.as_ref().is_none_or(|o| {
            o.server_version.is_none() || o.catalog.as_ref().is_none_or(|c| c.digest.is_none())
        }) {
            report.stop_reason =
                Some("Initial server/model identity unavailable; no inference was started".into());
            break;
        }
        report.cases[index].phase = Phase::Running;
        output.save(&mut report)?; // exact scenario checkpoint before fixture creation or generation
        let before = inference_runtime::inspect(&provider, model).await;
        let before = match before {
            Ok(before) => before,
            Err(error) => {
                report.cases[index].error = Some(clean(&error));
                report.cases[index].phase = Phase::Finished;
                output.save(&mut report)?;
                continue;
            }
        };
        let drift = report
            .manifest
            .initial_runtime
            .as_ref()
            .unwrap()
            .drift_reasons(&before);
        report.cases[index].before = Some(before);
        if !drift.is_empty() {
            report.cases[index].error = Some(clean(&drift.join("; ")));
            report.cases[index].phase = Phase::Finished;
            report.stop_reason =
                Some("Runtime/model identity changed; remaining cases were not started".into());
            output.save(&mut report)?;
            break;
        }
        if cancel.load(Ordering::SeqCst)
            || started.elapsed() >= Duration::from_secs(DEADLINE_SECONDS)
        {
            let reason =
                "Cohort cancelled or deadline reached during inspection; fixture was not started";
            report.cases[index].error = Some(reason.into());
            report.cases[index].phase = Phase::Finished;
            report.stop_reason = Some(reason.into());
            output.save(&mut report)?;
            break;
        }
        output.save(&mut report)?;
        let key = report.cases[index].key.clone();
        let fixture_path = output.artifacts.join(format!("case-{:02}", index + 1));
        let scenario = qualification_runner::run_scenario_with_cancel(
            &fixture_path,
            provider.clone().with_context_profile(key.profile),
            model,
            key.scenario,
            cancel.clone(),
        );
        tokio::pin!(scenario);
        let remaining = Duration::from_secs(DEADLINE_SECONDS).saturating_sub(started.elapsed());
        let result = tokio::select! {
            result = &mut scenario => result,
            _ = tokio::time::sleep(remaining) => { cancel.store(true, Ordering::SeqCst); scenario.await }
        };
        match result {
            Ok(result) => report.cases[index].result = Some(result),
            Err(error) => report.cases[index].error = Some(clean(&error)),
        }
        match inference_runtime::inspect(&provider, model).await {
            Ok(after) => report.cases[index].after = Some(after),
            Err(error) => report.cases[index].inspection_error = Some(clean(&error)),
        }
        let captured = recorder.snapshot()?;
        report.cases[index].requests = captured[cursor..].to_vec();
        cursor = captured.len();
        report.cases[index].phase = Phase::Finished;
        output.save(&mut report)?;
        eprintln!(
            "Qualification case {}/{}: {}",
            index + 1,
            report.cases.len(),
            if report.case_issues(&report.cases[index]).is_empty() {
                "accepted with complete observations"
            } else {
                "recorded; review limitations in report"
            }
        );
    }
    report.finished =
        report.cases.iter().all(|c| c.phase == Phase::Finished) && report.stop_reason.is_none();
    output.save(&mut report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "alfredo-report-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn report(&self) -> Report {
            let manifest = Manifest {
                schema_version: 1,
                endpoint_origin: "http://localhost:11434".into(),
                model: "fixture".into(),
                executable_sha256: "a".repeat(64),
                upstream_binary_pin_verified: false,
                repetitions: 1,
                capacity: 1,
                max_requests: MAX_REQUESTS,
                deadline_seconds: DEADLINE_SECONDS,
                structured_thinking: Some(false),
                fixtures: Scenario::ALL
                    .into_iter()
                    .map(qualification_runner::fixture_definition)
                    .collect(),
                schedule: schedule(1),
                initial_runtime: None,
                initial_error: Some("Fixture metadata unavailable".into()),
                artifact_directory: self.0.join("report.json.artifacts"),
            };
            let cases = manifest
                .schedule
                .iter()
                .map(|key| Case {
                    key: key.clone(),
                    phase: Phase::Pending,
                    before: None,
                    after: None,
                    result: None,
                    error: None,
                    inspection_error: None,
                    requests: vec![],
                })
                .collect();
            let mut report = Report {
                schema_version: 1,
                manifest_sha256: digest(&json(&manifest).unwrap()),
                manifest,
                cases,
                finished: false,
                stop_reason: None,
                sha256: String::new(),
            };
            report.seal().unwrap();
            report
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn interrupted_checkpoint_round_trips_without_replay_or_accepted_observations() {
        let fixture = Fixture::new();
        let mut report = fixture.report();
        let mut output = Output::new(&fixture.0.join("report.json")).unwrap();
        output.save(&mut report).unwrap();
        report.cases[0].phase = Phase::Running;
        output.save(&mut report).unwrap();
        let saved = read(&output.path).unwrap();
        assert_eq!(saved.cases[0].phase, Phase::Running);
        assert!(!saved.finished);
        assert!(saved
            .summary()
            .contains("1/8 attempted, 0 canonical accepted, 0 with complete"));
        assert!(fs::read_dir(&output.artifacts).unwrap().next().is_none());
        assert!(Output::new(&output.path).is_err());
    }
    #[test]
    fn failed_scenarios_remain_in_denominator_without_reviewed_latency() {
        let fixture = Fixture::new();
        let mut report = fixture.report();
        for case in &mut report.cases {
            case.phase = Phase::Finished;
            case.error = Some("Model failed".into());
        }
        report.finished = true;
        report.seal().unwrap();
        assert!(report
            .summary()
            .contains("8/8 attempted, 0 canonical accepted"));
        assert!(report
            .cases
            .iter()
            .all(|case| !report.case_issues(case).is_empty()));
    }
    #[test]
    fn validation_rejects_tampering_false_completion_and_cross_case_order_even_after_reseal() {
        let fixture = Fixture::new();
        let original = fixture.report();
        let mut report = original.clone();
        report.cases[0].key.repetition = 2;
        assert!(report.seal().is_err());
        let mut report = original.clone();
        report.finished = true;
        assert!(report.seal().is_err());
        let mut report = original.clone();
        report.cases[1].phase = Phase::Running;
        assert!(report.seal().is_err());
        let mut report = original.clone();
        report.manifest.upstream_binary_pin_verified = true;
        report.manifest_sha256 = digest(&json(&report.manifest).unwrap());
        assert!(report.seal().is_err());
        let mut report = original.clone();
        report.cases[0].error = Some("Forged failure on unstarted case".into());
        assert!(report.validate().is_err());
        let mut report = original;
        report.manifest.executable_sha256 = "b".repeat(64);
        assert!(report.validate().is_err());
    }
    #[test]
    fn existing_or_oversized_reports_are_not_overwritten_or_replayed() {
        let fixture = Fixture::new();
        let path = fixture.0.join("report.json");
        fs::write(&path, b"existing user report").unwrap();
        assert!(Output::new(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing user report");
        assert!(!fixture.0.join("report.json.artifacts").exists());
        assert!(read(&path).is_err());
        fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(read(&path).err().unwrap().contains("4 MiB"));
    }
    #[test]
    fn edited_or_replaced_owned_report_is_preserved_before_checkpoint_write() {
        for replace in [false, true] {
            let fixture = Fixture::new();
            let mut report = fixture.report();
            let mut output = Output::new(&fixture.0.join("report.json")).unwrap();
            output.save(&mut report).unwrap();
            if replace {
                fs::remove_file(&output.path).unwrap();
            }
            fs::write(&output.path, b"user replacement").unwrap();
            report.cases[0].phase = Phase::Running;
            assert!(output.save(&mut report).is_err());
            assert_eq!(fs::read(&output.path).unwrap(), b"user replacement");
            assert!(!output.artifacts.join("report.pending").exists());
        }
    }
    #[test]
    fn pair_order_is_explicit_and_alternates_without_inventing_cold_warm_labels() {
        let cases = schedule(3);
        assert_eq!(cases.len(), 24);
        assert_eq!(cases[0].profile, ContextProfile::Baseline);
        assert_eq!(cases[8].profile, ContextProfile::ContextCandidate);
        assert_eq!(cases[16].profile, ContextProfile::Baseline);
        for pair in cases.chunks_exact(2) {
            assert_eq!(pair[0].scenario, pair[1].scenario);
            assert_eq!(pair[0].repetition, pair[1].repetition);
            assert_ne!(pair[0].profile, pair[1].profile);
        }
    }
}
