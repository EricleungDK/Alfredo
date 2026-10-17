//! Ollama model-name comparison. Ollama treats `name` as `name:latest`.

/// The name with an explicit tag. Only the segment after the last `/` carries
/// a tag, so a `:` earlier in a registry host (`host:5000/user/model`) is not one.
pub fn canonical_model_name(name: &str) -> String {
    let last = name.rsplit('/').next().unwrap_or(name);
    if last.contains(':') {
        name.to_string()
    } else {
        format!("{name}:latest")
    }
}

/// Whether two names denote the same installed model. A bare name equals its
/// `:latest` form and no other tag.
pub fn same_model(a: &str, b: &str) -> bool {
    a == b || canonical_model_name(a) == canonical_model_name(b)
}

/// Whether `name` is in `catalog`, ignoring the implied `:latest` tag.
pub fn contains_model<S: AsRef<str>>(catalog: &[S], name: &str) -> bool {
    catalog.iter().any(|m| same_model(m.as_ref(), name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_name_is_latest() {
        assert!(same_model("qwen2.5-coder", "qwen2.5-coder:latest"));
        assert!(same_model("qwen2.5-coder:latest", "qwen2.5-coder"));
        assert!(same_model("llama3:8b", "llama3:8b"));
    }

    #[test]
    fn only_latest_is_implied() {
        assert!(!same_model("qwen2.5-coder", "qwen2.5-coder:14b"));
        assert!(!same_model("qwen2.5-coder:14b", "qwen2.5-coder:latest"));
        assert!(!same_model("llama3", "llama3x"));
    }

    #[test]
    fn namespaced_names_tag_the_last_segment() {
        assert!(same_model("user/model", "user/model:latest"));
        assert!(same_model(
            "registry/user/model",
            "registry/user/model:latest"
        ));
        assert!(same_model(
            "host:5000/user/model",
            "host:5000/user/model:latest"
        ));
        assert!(!same_model("user/model", "user/model:7b"));
        assert!(!same_model("user/model", "other/model"));
    }

    #[test]
    fn catalog_lookup() {
        let catalog = vec!["llama3:8b".to_string(), "qwen2.5-coder:latest".to_string()];
        assert!(contains_model(&catalog, "qwen2.5-coder"));
        assert!(!contains_model(&catalog, "llama3"));
    }
}
