use alfredo_tui::models::{canonical_model_name, same_model};

#[test]
fn surrounding_spaces_are_not_part_of_a_model_name() {
    assert_eq!(canonical_model_name(" llama3 "), "llama3:latest");
    assert!(same_model(" llama3:8b ", "llama3:8b"));
}
