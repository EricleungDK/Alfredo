//! One display form for a check argv, shared by every surface that shows it
//! (plan draft, task panel, agent view, chat context, commit message).
//! Display only: the argv that runs is never rebuilt from this text.

/// One check argv on one line. An argument that is empty or contains whitespace, a
/// quote, a backslash or `$` is single-quoted (an inner `'` becomes `'\''`), so two
/// argvs split differently never read the same.
pub(crate) fn check_line(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || "'\"\\$".contains(c))
            {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::check_line;

    fn line(argv: &[&str]) -> String {
        check_line(&argv.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn plain_arguments_stay_bare() {
        assert_eq!(line(&["python3", "-m", "unittest"]), "python3 -m unittest");
        assert_eq!(line(&[]), "");
    }

    #[test]
    fn empty_argument_is_two_quotes() {
        assert_eq!(line(&["echo", ""]), "echo ''");
        assert_eq!(line(&[""]), "''");
    }

    #[test]
    fn each_trigger_character_quotes_the_argument() {
        assert_eq!(line(&["a b"]), "'a b'");
        assert_eq!(line(&["a\tb"]), "'a\tb'");
        assert_eq!(line(&["a\nb"]), "'a\nb'");
        assert_eq!(line(&["a\"b"]), "'a\"b'");
        assert_eq!(line(&["a\\b"]), "'a\\b'");
        assert_eq!(line(&["a$b"]), "'a$b'");
        assert_eq!(line(&["it's"]), "'it'\\''s'");
        assert_eq!(line(&["'"]), "''\\'''");
    }

    #[test]
    fn different_splits_read_differently() {
        assert_ne!(
            line(&["python3", "-c", "import sys; sys.exit(1)"]),
            line(&["python3", "-c", "import", "sys;", "sys.exit(1)"])
        );
        assert_ne!(line(&["a b"]), line(&["a", "b"]));
        assert_ne!(line(&["a", ""]), line(&["a"]));
    }
}
