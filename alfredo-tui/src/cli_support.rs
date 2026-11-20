/// Selection-stage failures point at the selection history; a busy conversation is reported plainly.
pub(crate) fn launch_error_message(error: String, journal: &std::path::Path) -> String {
    if alfredo_tui::conversations::is_namespace_in_use(&error) {
        error
    } else {
        format!(
            "{error}. Inspect selection history at {}",
            journal.display()
        )
    }
}

/// Every flag `run` accepts; a `--chat` value that is one of these is a missing PROMPT.
pub(crate) const KNOWN_FLAGS: &[&str] = &[
    "--icons",
    "--no-motion",
    "--doctor",
    "--select",
    "--go",
    "--headless",
    "--output-format",
    "--resume",
    "--chat",
    "--approve",
    "--max-repairs",
    "--on-finish",
    "--qualify-inference",
    "--inspect-qualification",
    "--qualification-repetitions",
    "--workspace",
    "--state-dir",
    "--structured-thinking",
    "--worker-format",
    "--parallel-models",
    "--conversation",
    "--keep-alive",
    "--num-ctx",
    "--max-turns",
    "--connect-retries",
    "--mission",
    "--new-mission",
    "--model",
    "--endpoint",
    "--help",
    "-h",
    "--version",
    "-V",
];

/// Largest prompt `--chat -` accepts from stdin.
const STDIN_PROMPT_CAP: u64 = 256 * 1024;

/// Reads the `--chat -` prompt: UTF-8, at most `STDIN_PROMPT_CAP` bytes, not blank.
pub(crate) fn read_stdin_prompt(reader: impl std::io::Read) -> Result<String, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    // One byte past the cap tells "too big" from "exactly full" without buffering an endless stream.
    reader
        .take(STDIN_PROMPT_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("--chat - could not read stdin: {error}"))?;
    if bytes.len() as u64 > STDIN_PROMPT_CAP {
        return Err("--chat - prompt on stdin is over the 256 KiB limit".into());
    }
    let prompt =
        String::from_utf8(bytes).map_err(|_| "--chat - needs UTF-8 text on stdin".to_string())?;
    if prompt.trim().is_empty() {
        return Err("--chat - needs a nonempty prompt on stdin".into());
    }
    Ok(prompt)
}

#[cfg(test)]
mod tests {
    #[test]
    fn launch_errors_keep_the_selection_history_pointer_except_a_busy_conversation() {
        let journal = std::path::Path::new("/s/selections.json");
        let genuine =
            super::launch_error_message("Selection stopped after mission ready".into(), journal);
        assert!(
            genuine.ends_with("Inspect selection history at /s/selections.json"),
            "{genuine}"
        );
        let busy = alfredo_tui::conversations::namespace_in_use("default");
        assert_eq!(super::launch_error_message(busy.clone(), journal), busy);
    }
}
