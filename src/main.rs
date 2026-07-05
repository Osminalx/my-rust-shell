mod args;
mod commands;
mod completer;
mod jobs;
use rustyline::{CompletionType, Config, Editor, error::ReadlineError};
use std::sync::{Arc, Mutex};

use crate::{commands::ShellState, completer::ShellHelper};

fn main() {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .build();

    let shell_state = Arc::new(Mutex::new(ShellState::new()));

    if let Ok(histfile) = std::env::var("HISTFILE")
        && let Ok(content) = std::fs::read_to_string(&histfile)
        && let Ok(mut state) = shell_state.lock()
    {
        state.history.extend(
            content
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string()),
        );
    }

    let helper = ShellHelper::new(Arc::clone(&shell_state));

    let mut rl = Editor::with_config(config).expect("Failed to create editor");
    rl.set_helper(Some(helper));

    loop {
        if let Ok(mut state) = shell_state.lock() {
            state.jobs.poll();
        }

        match rl.readline("$ ") {
            Ok(command) => {
                let command = command.trim();
                if command.is_empty() {
                    continue;
                }

                rl.add_history_entry(command).ok();

                if let Ok(mut state) = shell_state.lock() {
                    state.history.push(command.to_string());
                }

                let variables = {
                    let state = shell_state.lock().expect("shell: failed to lock state");
                    state.variables.clone()
                };
                let mut pipeline = args::parse_pipeline(command, &variables);
                commands::execute_pipeline(&mut pipeline, Arc::clone(&shell_state));
            }
            Err(ReadlineError::Interrupted) => {
                continue;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                eprint!("Error: {err}");
                break;
            }
        }
    }
}
