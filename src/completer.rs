use crate::commands::ShellState;
use std::env;
use std::sync::{Arc, Mutex};

use crate::commands::{BUILTINS, get_path_cmd};
use rustyline::{
    Context, Helper,
    completion::{Candidate, Completer, FilenameCompleter},
    highlight::Highlighter,
    hint::Hinter,
    validate::Validator,
};

pub struct ShellHelper {
    file_completer: FilenameCompleter,
    pub state: Arc<Mutex<ShellState>>,
}

impl ShellHelper {
    pub fn new(state: Arc<Mutex<ShellState>>) -> Self {
        Self {
            file_completer: FilenameCompleter::new(),
            state,
        }
    }
}

pub struct CommandCandidate {
    display: String,
    replacement: String,
}

impl Candidate for CommandCandidate {
    fn display(&self) -> &str {
        &self.display
    }
    fn replacement(&self) -> &str {
        &self.replacement
    }
}

impl Completer for ShellHelper {
    type Candidate = CommandCandidate;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context,
    ) -> rustyline::Result<(usize, Vec<CommandCandidate>)> {
        let word = &line[..pos];

        if word.contains(' ') {
            // Command completion
            let mut parts = word.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("");

            let completer_path = {
                if let Ok(state) = self.state.lock() {
                    state.completions.get(cmd).cloned()
                } else {
                    eprintln!("shell: couldn't read state, exiting");
                    std::process::exit(1);
                }
            };

            if let Some(completer) = completer_path {
                let all_words: Vec<&str> = word.split_whitespace().collect();

                let current_word = if word.ends_with(' ') {
                    ""
                } else {
                    all_words.last().copied().unwrap_or("")
                };
                let prev_word = if word.ends_with(' ') {
                    all_words.last().copied().unwrap_or("")
                } else if all_words.len() >= 2 {
                    all_words[all_words.len() - 2]
                } else {
                    ""
                };

                let start = pos - current_word.len();

                let output = std::process::Command::new(&completer)
                    .env("COMP_LINE", line)
                    .env("COMP_POINT", pos.to_string())
                    .arg(cmd)
                    .arg(current_word)
                    .arg(prev_word)
                    .output();

                if let Ok(out) = output {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let mut candidates: Vec<CommandCandidate> = stdout
                        .lines()
                        .filter(|l| !l.is_empty())
                        .map(|l| CommandCandidate {
                            display: l.to_string(),
                            replacement: format!("{} ", l),
                        })
                        .collect();
                    candidates.sort_by(|a, b| a.display.cmp(&b.display));
                    return Ok((start, candidates));
                }
            }

            // FileName Completion
            let (start, pairs) = self.file_completer.complete_path(line, pos)?;
            let candidates = pairs
                .into_iter()
                .map(|p| {
                    let replacement = p.replacement().to_string();
                    let replacement = if replacement.ends_with('/') {
                        replacement
                    } else {
                        format!("{} ", replacement)
                    };

                    let display = if replacement.ends_with('/') {
                        replacement.clone()
                    } else {
                        replacement.trim_end().to_string()
                    };
                    CommandCandidate {
                        display,
                        replacement,
                    }
                })
                .collect();
            Ok((start, candidates))
        } else {
            let mut candidates = complete_command(word);

            candidates.sort_by(|a, b| a.display.cmp(&b.display));
            Ok((0, candidates))
        }
    }
}

fn complete_command(word: &str) -> Vec<CommandCandidate> {
    let mut candidates: Vec<CommandCandidate> = vec![];

    for builtin in BUILTINS {
        if builtin.starts_with(word) {
            candidates.push(CommandCandidate {
                display: builtin.to_string(),
                replacement: format!("{} ", builtin),
            });
        }
    }

    if let Ok(path_var) = env::var("PATH") {
        for dir in path_var.split(':') {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with(word) {
                    // Reutilizamos is_executable a través de get_path_cmd
                    if get_path_cmd(&name_str).is_some() {
                        // Evitar duplicados con builtins
                        if !candidates.iter().any(|p| p.display == name_str.as_ref()) {
                            candidates.push(CommandCandidate {
                                display: name_str.to_string(),
                                replacement: format!("{} ", name_str),
                            });
                        }
                    }
                }
            }
        }
    }
    candidates
}

impl Hinter for ShellHelper {
    type Hint = String;
}

impl Highlighter for ShellHelper {}
impl Validator for ShellHelper {}
impl Helper for ShellHelper {}
