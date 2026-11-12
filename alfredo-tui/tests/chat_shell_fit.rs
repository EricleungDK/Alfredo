//! Issue #692: `run_command` reports cut to a tight chat budget keep the exit-code
//! header and the ends of stderr and stdout, each cut section marked.
use alfredo_tui::chat_tools::fit_run_command;

fn sample(header: &str, line: &str, out_lines: usize, err: &str) -> String {
    let out: String = (0..out_lines)
        .map(|i| format!("{line} {i}\n"))
        .collect::<String>()
        .trim_end()
        .to_string();
    format!("{header}\nstdout:\n{out}\nstderr:\n{err}")
}

#[test]
fn fit_returns_a_result_that_fits_unchanged() {
    let text = sample("exit code 1 \u{b7} sandboxed", "line", 5, "boom");
    assert_eq!(fit_run_command(text.clone(), text.len()), text);
    assert_eq!(fit_run_command(text.clone(), 10_000), text);
}

#[test]
fn fit_leaves_a_result_of_exactly_the_limit_alone_even_with_a_long_header() {
    let text = sample(&"h".repeat(300), "line", 3, "boom");
    assert_eq!(fit_run_command(text.clone(), text.len()), text);
    assert_ne!(fit_run_command(text.clone(), text.len() - 1), text);
}

#[test]
fn fit_keeps_header_stderr_and_the_last_stdout_line_with_markers() {
    let text = sample(
        "exit code 1 \u{b7} sandboxed",
        "progress line",
        600,
        "ERROR: real",
    );
    let fitted = fit_run_command(text.clone(), 1500);
    assert!(fitted.len() <= 1500, "{}", fitted.len());
    assert!(
        fitted.starts_with("exit code 1 \u{b7} sandboxed\nstdout:\n"),
        "{fitted}"
    );
    assert!(fitted.ends_with("\nstderr:\nERROR: real"), "{fitted}");
    assert!(fitted.contains("progress line 599"), "{fitted}");
    assert!(fitted.contains("earlier bytes cut]"), "{fitted}");
    assert!(!fitted.contains("progress line 0\n"), "{fitted}");
}

#[test]
fn fit_marks_a_cut_stderr_and_gives_it_a_third_of_the_room() {
    let err = "e".repeat(5000);
    let text = sample("exit code 1", "out", 2000, &err);
    let fitted = fit_run_command(text, 900);
    let (out, err) = fitted.split_once("\nstderr:\n").unwrap();
    assert!(fitted.len() <= 900, "{}", fitted.len());
    assert!(
        err.starts_with("[\u{2026} ") && err.contains(" earlier bytes cut]\n"),
        "{err}"
    );
    assert!(
        err.len() >= 200,
        "stderr keeps about a third: {}",
        err.len()
    );
    assert!(out.contains("earlier bytes cut]"), "{out}");
    assert!(out.ends_with("out 1999"), "{out}");
}

#[test]
fn fit_never_cuts_inside_a_character() {
    for limit in 256..700 {
        let text = sample(
            "exit code 2",
            "\u{e9}\u{e9}\u{e9} \u{4e2d}\u{6587}",
            400,
            &"\u{e9}".repeat(900),
        );
        let fitted = fit_run_command(text, limit);
        assert!(fitted.len() <= limit, "{limit}: {}", fitted.len());
        assert!(fitted.contains("earlier bytes cut]"), "{limit}");
    }
}

#[test]
fn fit_keeps_the_timeout_and_cancel_headers() {
    for header in [
        "error: timed out after 7s (host); the command and its child processes were killed",
        "cancelled (sandboxed); the command and its child processes were killed",
    ] {
        let text = sample(header, "tick", 800, "last words");
        let fitted = fit_run_command(text, 400);
        assert!(fitted.len() <= 400, "{}", fitted.len());
        assert!(fitted.starts_with(header), "{fitted}");
        assert!(fitted.contains("tick 799"), "{fitted}");
        assert!(fitted.ends_with("last words"), "{fitted}");
    }
}

#[test]
fn fit_cuts_an_oversized_header_with_a_marker_and_keeps_the_sections() {
    let header = format!("exit code 1\n{}", "n".repeat(2000));
    let text = sample(&header, "x", 300, "bad");
    let fitted = fit_run_command(text, 600);
    assert!(fitted.len() <= 600, "{}", fitted.len());
    assert!(fitted.starts_with("exit code 1\n"), "{fitted}");
    assert!(fitted.contains("[\u{2026} rest cut]"), "{fitted}");
    assert!(fitted.ends_with("\nstderr:\nbad"), "{fitted}");
}

#[test]
fn fit_falls_back_to_the_plain_cap_for_text_without_sections() {
    let text = "error: nope ".repeat(200);
    let fitted = fit_run_command(text.clone(), 300);
    assert_eq!(
        fitted,
        format!("{}\n[truncated: 300 of 2400 bytes]", &text[..300])
    );
    assert!(!fitted.contains("earlier bytes cut"), "{fitted}");
}

#[test]
fn fit_cuts_an_oversized_header_on_a_character_boundary_and_sizes_the_sections_after_it() {
    // 400-byte header of two-byte characters; limit 300 gives the header 150 bytes, 15
    // of them the marker, so 135 would split a character and the cut falls at 134.
    let text = format!(
        "{}\nstdout:\n{}\nstderr:\n{}",
        "\u{e9}".repeat(200),
        "o".repeat(500),
        "e".repeat(500)
    );
    let fitted = fit_run_command(text, 300);
    let expected = format!(
        "{}\n[\u{2026} rest cut]\nstdout:\n[\u{2026} 439 earlier bytes cut]\n{}\nstderr:\n[\u{2026} 484 earlier bytes cut]\n{}",
        "\u{e9}".repeat(67),
        "o".repeat(61),
        "e".repeat(16)
    );
    assert_eq!(fitted, expected);
    assert_eq!(fitted.len(), 300);
}

#[test]
fn fit_gives_a_cut_stdout_exactly_the_room_stderr_leaves() {
    // room = 100 - (1 + 9 + 9) = 81; stderr needs 3, stdout gets 78: a 28-byte marker
    // and the last 50 bytes of the 200-byte stdout.
    let stdout: String = (0..200)
        .map(|i| char::from(b'0' + (i % 10) as u8))
        .collect();
    let text = format!("H\nstdout:\n{stdout}\nstderr:\nerr");
    let fitted = fit_run_command(text, 100);
    let expected = format!(
        "H\nstdout:\n[\u{2026} 150 earlier bytes cut]\n{}\nstderr:\nerr",
        &stdout[150..]
    );
    assert_eq!(fitted, expected);
    assert_eq!(fitted.len(), 100);
}

#[test]
fn fit_cuts_a_long_stderr_to_the_room_stdout_leaves() {
    // room = 200 - 19 = 181; stdout (100) fits whole, so stderr gets 81: a 27-byte
    // marker and its last 53 bytes.
    let text = format!(
        "H\nstdout:\n{}\nstderr:\n{}",
        "a".repeat(100),
        "b".repeat(100)
    );
    let fitted = fit_run_command(text, 200);
    let expected = format!(
        "H\nstdout:\n{}\nstderr:\n[\u{2026} 47 earlier bytes cut]\n{}",
        "a".repeat(100),
        "b".repeat(53)
    );
    assert_eq!(fitted, expected);
    assert_eq!(fitted.len(), 199);
}
