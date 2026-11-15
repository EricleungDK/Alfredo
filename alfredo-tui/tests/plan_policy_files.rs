use alfredo_tui::tasks::WorkPolicy;

fn policy(files: &[&str]) -> WorkPolicy {
    WorkPolicy {
        files: files.iter().map(|file| (*file).into()).collect(),
        check: vec!["true".into()],
    }
}

#[test]
fn policy_files_refuse_secret_paths() {
    for p in [".env", "config/id_rsa", ".aws/credentials", "deploy/server.key"] {
        let policy = WorkPolicy {
            files: vec![p.into()],
            check: vec!["true".into()],
        };
        let error = policy
            .validate()
            .expect_err(&format!("policy accepted secret path {p}"));
        assert!(error.contains(p), "refusal must name the path: {error}");
    }
}

#[test]
fn policy_files_refuse_hidden_characters() {
    for p in ["src/app\u{200b}.py", "a\u{202e}txt.py"] {
        let policy = WorkPolicy {
            files: vec![p.into()],
            check: vec!["true".into()],
        };
        assert!(
            policy.validate().is_err(),
            "policy accepted hidden char in {p:?}"
        );
    }
}

#[test]
fn policy_files_name_the_file_position_and_code_point_of_a_hidden_character() {
    let error = policy(&["src/ok.py", "a\u{202e}txt.py"])
        .validate()
        .unwrap_err();
    assert_eq!(
        error,
        "Policy file 2 contains hidden character U+202E; remove invisible or bidirectional characters"
    );
    let error = policy(&["src/app\u{200b}.py"]).validate().unwrap_err();
    assert_eq!(
        error,
        "Policy file 1 contains hidden character U+200B; remove invisible or bidirectional characters"
    );
}

#[test]
fn policy_files_accept_ordinary_paths_including_env_templates() {
    for p in ["src/main.rs", "docs/é.md", ".env.example"] {
        assert!(
            policy(&[p]).validate().is_ok(),
            "ordinary path {p} was refused"
        );
    }
}

#[test]
fn policy_files_accept_names_that_only_resemble_secrets() {
    for p in ["src/environment.rs", "docs/keyboard.md", "src/keys.rs"] {
        assert!(
            policy(&[p]).validate().is_ok(),
            "non-secret path {p} was refused"
        );
    }
}
