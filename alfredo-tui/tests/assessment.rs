use alfredo_tui::{
    assessment::{Assessment, Criterion},
    task_control::parse,
    tasks::Action,
};
fn assessment() -> Assessment {
    Assessment {
        accept: true,
        reason: "Reviewed the implementation and test".into(),
        criteria: vec![Criterion {
            criterion: 1,
            met: true,
            note: "check asserts answer equals 42".into(),
        }],
    }
}
#[test]
fn review_command_is_typed_and_requires_exact_observable_criterion_coverage() {
    let review = assessment();
    let json = serde_json::to_string(&review).unwrap();
    assert_eq!(
        parse(&format!("/review 7 {json}"), "model").unwrap(),
        Action::Assess {
            task: 7,
            assessment: review.clone()
        }
    );
    review.validate_contract(&["Answer is 42".into()]).unwrap();
    assert!(review.validate_contract(&[]).is_err());
    assert!(review
        .validate_contract(&["Answer is 42".into(), "No mutation".into()])
        .is_err());
    assert!(parse("/review 7 {}", "model").is_err());
    assert!(parse("/review nope {}", "model").is_err());
    let mut unknown = serde_json::to_value(review).unwrap();
    unknown["authority"] = true.into();
    assert!(parse(&format!("/review 7 {unknown}"), "model").is_err());
}
#[test]
fn review_rejects_unmet_acceptance_duplicates_control_characters_and_unbounded_notes() {
    let mut unmet = assessment();
    unmet.criteria[0].met = false;
    assert!(unmet.validate().is_err());
    unmet.accept = false;
    unmet.validate_contract(&["Answer is 42".into()]).unwrap();
    for number in [0, 2, u64::MAX] {
        let mut bad = assessment();
        bad.criteria[0].criterion = number;
        assert!(bad.validate().is_err());
    }
    let mut duplicate = assessment();
    duplicate.criteria.push(duplicate.criteria[0].clone());
    assert!(duplicate.validate().is_err());
    for text in [" ".into(), "line\nbreak".into(), "界".repeat(400)] {
        let mut bad = assessment();
        bad.criteria[0].note = text;
        assert!(bad.validate().is_err());
    }
    let mut bad = assessment();
    bad.reason = "x".repeat(2049);
    assert!(bad.validate().is_err());
    let mut legacy = assessment();
    legacy.criteria.clear();
    legacy.validate_contract(&[]).unwrap();
}

#[test]
fn five_outcomes_are_typed_and_limitations_cannot_waive_unmet_criteria() {
    use alfredo_tui::assessment::{Decision, Outcome};
    for outcome in [
        Outcome::Approved,
        Outcome::ApprovedWithLimitations,
        Outcome::NeedsRepair,
        Outcome::NeedsHumanReview,
        Outcome::Rejected,
    ] {
        let decision = Decision {
            failure: None,
            risk: None,
            outcome,
            reason: "Explicit reviewed outcome".into(),
            criteria: assessment().criteria,
            limitations: if outcome == Outcome::ApprovedWithLimitations {
                vec!["Performance beyond fixtures is unmeasured".into()]
            } else {
                vec![]
            },
        };
        decision
            .validate_contract(&["Answer is 42".into()])
            .unwrap();
        let command = format!("/review 7 {}", serde_json::to_string(&decision).unwrap());
        assert_eq!(
            parse(&command, "model").unwrap(),
            if decision.proposes_repair() {
                Action::ReviewAndRepair {
                    task: 7,
                    decision: decision.clone(),
                }
            } else {
                Action::Decide {
                    task: 7,
                    decision: decision.clone(),
                }
            }
        );
        let mut wrong = decision.clone();
        wrong.criteria[0].met = false;
        assert_eq!(wrong.validate().is_err(), outcome.approves());
        let mut wrong = decision;
        wrong.limitations = vec!["unexpected".into()];
        assert_eq!(
            wrong.validate().is_ok(),
            outcome == Outcome::ApprovedWithLimitations
        );
    }
    let mut limited = Decision {
        failure: None,
        risk: None,
        outcome: Outcome::ApprovedWithLimitations,
        reason: "Reason".into(),
        criteria: vec![],
        limitations: vec![],
    };
    assert!(limited.validate().is_err());
    for limits in [
        vec![" ".into()],
        vec!["same".into(), " same ".into()],
        vec!["界".repeat(400)],
        (0..9).map(|n| format!("limit {n}")).collect(),
    ] {
        limited.limitations = limits;
        assert!(limited.validate().is_err());
    }
    assert!(parse(
        r#"/review 7 {"outcome":"approved","accept":true,"reason":"x","criteria":[]}"#,
        "model"
    )
    .is_err());
}

#[test]
fn typed_risk_requires_human_resolution_and_legacy_absence_stays_unclassified() {
    use alfredo_tui::assessment::{Decision, Outcome, ReviewRisk};
    for risk in [
        ReviewRisk::Critical,
        ReviewRisk::Security,
        ReviewRisk::MergeRisk,
    ] {
        for outcome in [
            Outcome::Approved,
            Outcome::ApprovedWithLimitations,
            Outcome::NeedsRepair,
            Outcome::NeedsHumanReview,
            Outcome::Rejected,
        ] {
            let decision = Decision {
                failure: None,
                risk: Some(risk),
                outcome,
                reason: "Risk found in inspected evidence".into(),
                criteria: vec![],
                limitations: if outcome == Outcome::ApprovedWithLimitations {
                    vec!["Remaining limitation".into()]
                } else {
                    vec![]
                },
            };
            let json = serde_json::to_string(&decision).unwrap();
            assert_eq!(
                parse(&format!("/review 1 {json}"), "model").is_err(),
                outcome.approves()
            );
            assert!(decision.requires_human_review());
            assert!(decision.summary().contains(risk.label()));
        }
    }
    let old = r#"{"outcome":"rejected","reason":"Old review","criteria":[]}"#;
    let parsed: Decision = serde_json::from_str(old).unwrap();
    assert!(parsed.risk.is_none());
    assert!(!parsed.requires_human_review());
    assert!(!serde_json::to_string(&parsed).unwrap().contains("risk"));
    for risk in ["unknown", "none", "Security", "architecture"] {
        let mut value = serde_json::to_value(&parsed).unwrap();
        value["risk"] = risk.into();
        assert!(parse(&format!("/review 1 {value}"), "model").is_err());
    }
}

#[test]
fn architecture_failure_routes_separately_and_risk_retains_precedence() {
    let raw = r#"/review 2 {"outcome":"needs-repair","failure":"architecture","reason":"Boundary is wrong","criteria":[]}"#;
    assert!(matches!(
        parse(raw, "worker").unwrap(),
        Action::ReviewArchitecture { task: 2, .. }
    ));
    let risky = raw.replace(
        "\"failure\":\"architecture\"",
        "\"failure\":\"architecture\",\"risk\":\"security\"",
    );
    assert!(matches!(
        parse(&risky, "worker").unwrap(),
        Action::Decide { task: 2, .. }
    ));
    assert!(parse(&raw.replace("needs-repair", "approved"), "worker").is_err());
}
