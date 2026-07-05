use std::collections::HashMap;

enum ParseState {
    Unquoted,
    SingleQuoted,
    DoubleQuoted,
}

const DOUBLE_QUOTE_ESCAPABLE: &[char] = &['"', '\\', '$', '`', '\n'];

/// A segment of a token with quoting context preserved.
///
/// During tokenization, text from single-quoted or escaped contexts
/// becomes `Literal` — `$var` in these segments is NEVER expanded.
/// Text from unquoted or double-quoted contexts becomes `Expandable` —
/// `$var` IS expanded during the second pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Literal(String),
    Expandable(String),
}

#[derive(Debug, Clone)]
pub enum RedirectMode {
    Overwrite,
    Append,
}

#[derive(Debug, Clone)]
pub struct Redirect {
    pub path: String,
    pub mode: RedirectMode,
}

#[derive(Debug)]
pub struct ParsedCommand {
    pub args: Vec<String>,
    pub stdout_redirect: Option<Redirect>,
    pub stderr_redirect: Option<Redirect>,
    pub background: bool,
}

#[derive(Debug)]
pub struct Pipeline {
    pub commands: Vec<ParsedCommand>,
}

pub fn parse_pipeline(raw: &str, vars: &HashMap<String, String>) -> Pipeline {
    let segments = split_by_pipe(raw);
    let commands = segments.iter().map(|s| parse_args(s, vars)).collect();
    Pipeline { commands }
}

pub fn parse_args(raw_args: &str, vars: &HashMap<String, String>) -> ParsedCommand {
    let (tokens, stdout_redirect, stderr_redirect, background) = tokenize(raw_args);
    let args = expand_tokens(&tokens, vars);
    ParsedCommand {
        args,
        stdout_redirect,
        stderr_redirect,
        background,
    }
}

// ── Tokenization (Pass 1) ────────────────────────────────────────────

/// Tokenize a raw shell command string, preserving which parts of each
/// token are expandable vs literal. Pure — no external state needed.
fn tokenize(raw_args: &str) -> (Vec<Vec<Segment>>, Option<Redirect>, Option<Redirect>, bool) {
    let mut raw_tokens: Vec<Vec<Segment>> = vec![];
    let mut current_parts: Vec<Segment> = vec![];
    let mut current_buf = String::new();
    let mut has_content = false;
    let mut state = ParseState::Unquoted;
    let mut stdout_redirect: Option<Redirect> = None;
    let mut stderr_redirect: Option<Redirect> = None;
    let mut chars = raw_args.chars().peekable();
    let mut background = false;

    while let Some(c) = chars.next() {
        match (&state, c) {
            // ── Redirect ──────────────────────────────────────────
            (ParseState::Unquoted, '>') => {
                flush_buf(&mut current_parts, &mut current_buf, true);

                let is_2 = current_parts.len() == 1
                    && current_buf.is_empty()
                    && match &current_parts[0] {
                        Segment::Literal(s) | Segment::Expandable(s) => s == "2",
                    };
                let is_1 = current_parts.len() == 1
                    && current_buf.is_empty()
                    && match &current_parts[0] {
                        Segment::Literal(s) | Segment::Expandable(s) => s == "1",
                    };

                if has_content && !is_1 && !is_2 {
                    raw_tokens.push(std::mem::take(&mut current_parts));
                } else {
                    current_parts.clear();
                }
                has_content = false;

                let mode = if chars.peek() == Some(&'>') {
                    chars.next();
                    RedirectMode::Append
                } else {
                    RedirectMode::Overwrite
                };

                while chars.peek() == Some(&' ') {
                    chars.next();
                }

                let mut path = String::new();
                for pc in chars.by_ref() {
                    if pc == ' ' {
                        break;
                    }
                    path.push(pc);
                }

                if !path.is_empty() {
                    let redirect = Redirect { path, mode };
                    if is_2 {
                        stderr_redirect = Some(redirect);
                    } else {
                        stdout_redirect = Some(redirect);
                    }
                }
            }

            // ── Background ─────────────────────────────────────────
            (ParseState::Unquoted, '&') => {
                flush_buf(&mut current_parts, &mut current_buf, true);
                if chars.peek() != Some(&'&') {
                    background = true;
                } else {
                    chars.next();
                }
            }

            // ── Escape in unquoted: emitted char is literal ────────
            (ParseState::Unquoted, '\\') => {
                if let Some(escaped) = chars.next() {
                    // Flush any expandable content collected so far
                    flush_buf(&mut current_parts, &mut current_buf, true);
                    // The escaped char is always literal
                    current_buf.push(escaped);
                    flush_buf(&mut current_parts, &mut current_buf, false);
                }
                has_content = true;
            }

            // ── Escape in double-quoted ────────────────────────────
            (ParseState::DoubleQuoted, '\\') => {
                match chars.peek().copied() {
                    Some(next) if DOUBLE_QUOTE_ESCAPABLE.contains(&next) => {
                        chars.next();
                        // The escaped char is literal
                        flush_buf(&mut current_parts, &mut current_buf, true);
                        current_buf.push(next);
                        flush_buf(&mut current_parts, &mut current_buf, false);
                    }
                    _ => current_buf.push('\\'),
                }
                has_content = true;
            }

            // ── Single quotes: flip to literal context ─────────────
            (ParseState::Unquoted, '\'') => {
                // Flush whatever we had in expandable context
                flush_buf(&mut current_parts, &mut current_buf, true);
                state = ParseState::SingleQuoted;
                has_content = true;
            }
            (ParseState::SingleQuoted, '\'') => {
                // Flush the single-quoted content as literal
                flush_buf(&mut current_parts, &mut current_buf, false);
                state = ParseState::Unquoted;
            }

            // ── Double quotes: same expandability as unquoted ─────
            (ParseState::Unquoted, '"') => {
                state = ParseState::DoubleQuoted;
                has_content = true;
            }
            (ParseState::DoubleQuoted, '"') => {
                state = ParseState::Unquoted;
            }

            // ── Token boundary ─────────────────────────────────────
            (ParseState::Unquoted, ' ') => {
                if has_content {
                    flush_buf(&mut current_parts, &mut current_buf, true);
                    raw_tokens.push(std::mem::take(&mut current_parts));
                    has_content = false;
                }
            }

            // ── Regular character ──────────────────────────────────
            (_, c) => {
                current_buf.push(c);
                has_content = true;
            }
        }
    }

    // ── End of input: flush remaining ────────────────────────────────
    if has_content {
        let expandable = !matches!(state, ParseState::SingleQuoted);
        flush_buf(&mut current_parts, &mut current_buf, expandable);
        raw_tokens.push(current_parts);
    }

    (raw_tokens, stdout_redirect, stderr_redirect, background)
}

/// Drain `buf` into a `Segment` and push it onto `parts`.
/// `expandable` controls whether the segment is `Expandable` or `Literal`.
fn flush_buf(parts: &mut Vec<Segment>, buf: &mut String, expandable: bool) {
    if !buf.is_empty() {
        let seg = if expandable {
            Segment::Expandable(std::mem::take(buf))
        } else {
            Segment::Literal(std::mem::take(buf))
        };
        parts.push(seg);
    }
}

// ── Variable Expansion (Pass 2) ──────────────────────────────────────

/// Expand `$var` references inside every `Expandable` segment.
/// `Literal` segments are passed through verbatim.
pub fn expand_tokens(raw_tokens: &[Vec<Segment>], vars: &HashMap<String, String>) -> Vec<String> {
    raw_tokens
        .iter()
        .filter_map(|parts| {
            let mut expanded = String::new();
            for seg in parts {
                match seg {
                    Segment::Literal(s) => expanded.push_str(s),
                    Segment::Expandable(s) => expand_string(s, vars, &mut expanded),
                }
            }
            if expanded.is_empty() && !parts.is_empty() {
                None
            } else {
                Some(expanded)
            }
        })
        .collect()
}

/// Scan `s` for `$identifier` patterns and replace them.
fn expand_string(s: &str, vars: &HashMap<String, String>, result: &mut String) {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek() {
                Some('{') => {
                    chars.next();
                    let mut name = String::new();
                    let mut closed = false;
                    for c in chars.by_ref() {
                        if c == '}' {
                            closed = true;
                            break;
                        }

                        name.push(c);
                    }

                    if closed {
                        let value = vars.get(&name).map(|s| s.as_str()).unwrap_or("");
                        result.push_str(value);
                    } else {
                        result.push_str("${");
                        result.push_str(&name);
                    }
                }

                Some(c) if c.is_alphabetic() || *c == '_' => {
                    let mut name = String::new();
                    name.push(*c);
                    chars.next();
                    while let Some(c) = chars.peek() {
                        if c.is_alphanumeric() || *c == '_' {
                            name.push(*c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let value = vars.get(&name).map(|s| s.as_str()).unwrap_or("");
                    result.push_str(value);
                }
                _ => {
                    result.push('$');
                }
            }
        } else {
            result.push(c);
        }
    }
}

// ── Pipe splitting (unchanged) ───────────────────────────────────────

fn split_by_pipe(raw: &str) -> Vec<String> {
    let mut segments: Vec<String> = vec![];
    let mut current = String::new();
    let mut state = ParseState::Unquoted;
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        match (&state, c) {
            (ParseState::Unquoted, '|') => {
                if chars.peek() == Some(&'|') {
                    chars.next();
                    current.push_str("||");
                } else {
                    segments.push(std::mem::take(&mut current));
                }
            }
            (ParseState::Unquoted, '\'') => {
                state = ParseState::SingleQuoted;
                current.push(c);
            }
            (ParseState::SingleQuoted, '\'') => {
                state = ParseState::Unquoted;
                current.push(c);
            }
            (ParseState::Unquoted, '"') => {
                state = ParseState::DoubleQuoted;
                current.push(c);
            }
            (ParseState::DoubleQuoted, '"') => {
                state = ParseState::Unquoted;
                current.push(c);
            }
            (ParseState::Unquoted, '\\') => {
                current.push(c);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            _ => current.push(c),
        }
    }

    if !current.trim().is_empty() {
        segments.push(current);
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars_map() -> HashMap<String, String> {
        let mut v = HashMap::new();
        v.insert("var".to_string(), "hello".to_string());
        v.insert("var2".to_string(), "world".to_string());
        v.insert("_private".to_string(), "secret".to_string());
        v
    }

    #[test]
    fn test_quoted_string() {
        let vars = HashMap::new();
        let result = parse_args("'holoa mundo; aserfafds'", &vars);
        assert_eq!(
            result.args,
            vec!["holoa mundo; aserfafds"],
            "Input: 'holoa mundo; aserfafds'"
        );
    }

    #[test]
    fn test_unquoted() {
        let vars = HashMap::new();
        let result = parse_args("holoa mundo", &vars);
        assert_eq!(result.args, vec!["holoa", "mundo"]);
    }

    #[test]
    fn test_mixed_concatenates() {
        let vars = HashMap::new();
        let result = parse_args("ho'la'", &vars);
        assert_eq!(
            result.args,
            vec!["hola"],
            "Mixed quoting should concatenate: ho'la'"
        );
    }

    #[test]
    fn test_no_closing_quote() {
        let vars = HashMap::new();
        let result = parse_args("'holoa mundo", &vars);
        assert_eq!(
            result.args,
            vec!["holoa mundo"],
            "Unclosed quote: 'holoa mundo"
        );
    }

    #[test]
    fn test_leading_space_quoted() {
        let vars = HashMap::new();
        let result = parse_args(" 'holoa mundo; aserfafds'", &vars);
        assert_eq!(result.args, vec!["holoa mundo; aserfafds"], "Leading space");
    }

    #[test]
    fn test_unclosed_and_unquoted() {
        let vars = HashMap::new();
        let result = parse_args("holoa mundo", &vars);
        assert_eq!(result.args, vec!["holoa", "mundo"], "Unquoted: holoa mundo");
    }

    #[test]
    fn test_multiple_quoted_groups() {
        let vars = HashMap::new();
        let result = parse_args("'a' 'b'", &vars);
        assert_eq!(result.args, vec!["a", "b"], "Multiple quoted: 'a' 'b'");
    }

    #[test]
    fn test_adjacent_quoted_groups_concatenate() {
        let vars = HashMap::new();
        let result = parse_args("'a''b'", &vars);
        assert_eq!(
            result.args,
            vec!["ab"],
            "Adjacent quoted should concatenate: 'a''b'"
        );
    }

    #[test]
    fn test_quoted_with_trailing() {
        let vars = HashMap::new();
        let result = parse_args("'a' b", &vars);
        assert_eq!(result.args, vec!["a", "b"], "Quoted then unquoted: 'a' b");
    }

    #[test]
    fn test_leads_with_unquoted_then_quoted_trailing() {
        let vars = HashMap::new();
        let result = parse_args("a 'b'", &vars);
        assert_eq!(result.args, vec!["a", "b"], "Unquoted then quoted: a 'b'");
    }

    #[test]
    fn test_empty_quotes() {
        let vars = HashMap::new();
        let result = parse_args("''", &vars);
        assert_eq!(result.args, vec![""], "Empty quotes: ''");
    }

    #[test]
    fn test_multiple_spaces() {
        let vars = HashMap::new();
        let result = parse_args("  a   b  ", &vars);
        assert_eq!(result.args, vec!["a", "b"], "Multiple spaces: '  a   b  '");
    }

    #[test]
    fn test_quotes_and_spaces_mix() {
        let vars = HashMap::new();
        let result = parse_args("echo 'hola mundo' 'foo bar'", &vars);
        assert_eq!(
            result.args,
            vec!["echo", "hola mundo", "foo bar"],
            "Mixed quotes and spaces"
        );
    }

    #[test]
    fn test_trailing_unclosed_quote() {
        let vars = HashMap::new();
        let result = parse_args("echo 'hola", &vars);
        assert_eq!(
            result.args,
            vec!["echo", "hola"],
            "Trailing unclosed quote: echo 'hola"
        );
    }

    // ── Variable expansion tests ─────────────────────────────────────

    #[test]
    fn test_expand_unquoted_var() {
        let vars = vars_map();
        let result = parse_args("echo $var", &vars);
        assert_eq!(result.args, vec!["echo", "hello"]);
    }

    #[test]
    fn test_expand_multiple_vars() {
        let vars = vars_map();
        let result = parse_args("echo $var $var2", &vars);
        assert_eq!(result.args, vec!["echo", "hello", "world"]);
    }

    #[test]
    fn test_single_quoted_prevents_expansion() {
        let vars = vars_map();
        let result = parse_args("echo '$var'", &vars);
        assert_eq!(result.args, vec!["echo", "$var"]);
    }

    #[test]
    fn test_double_quoted_expands() {
        let vars = vars_map();
        let result = parse_args("echo \"$var\"", &vars);
        assert_eq!(result.args, vec!["echo", "hello"]);
    }

    #[test]
    fn test_var_brace_expansion() {
        let vars = vars_map();
        let result = parse_args("echo pre${var}post", &vars);
        assert_eq!(result.args, vec!["echo", "prehellopost"]);
    }

    #[test]
    fn test_var_adjacent_to_text() {
        let vars = vars_map();
        let result = parse_args("echo pre$var", &vars);
        assert_eq!(result.args, vec!["echo", "prehello"]);
    }

    #[test]
    fn test_var_underscore_prefix() {
        let vars = vars_map();
        let result = parse_args("echo $_private", &vars);
        assert_eq!(result.args, vec!["echo", "secret"]);
    }

    #[test]
    fn test_undefined_var_removes_word() {
        let vars = HashMap::new();
        let result = parse_args("echo $undefined", &vars);
        // POSIX: unset variable expands to nothing, word is removed
        assert_eq!(result.args, vec!["echo"]);
    }

    #[test]
    fn test_mixed_literal_and_expandable() {
        let vars = vars_map();
        let result = parse_args("echo 'literal_$var'_and_$var", &vars);
        // 'literal_$var' → Literal("literal_$var")
        // _and_ → Expandable("_and_")
        // $var → Expandable("$var") → "hello"
        // Result: "literal_$var_and_hello"
        assert_eq!(result.args, vec!["echo", "literal_$var_and_hello"]);
    }
}
