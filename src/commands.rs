use std::collections::HashMap;
use std::env::{self, current_dir, var};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::jobs::JobStatus;
use crate::{
    args::{ParsedCommand, Redirect, RedirectMode},
    jobs::JobTable,
};

enum PipelineChild {
    Process(std::process::Child),
    Thread(std::thread::JoinHandle<()>),
}

#[derive(Debug, Default)]
pub struct CmdOutput {
    pub stdout: String,
    pub stderr: String,
}

enum ExecResult {
    Completed(CmdOutput),
    Background,
}

pub struct ShellState {
    pub jobs: JobTable,
    pub completions: HashMap<String, String>,
    pub history: Vec<String>,
    pub history_append: usize,
    pub variables: HashMap<String, String>,
}

impl ShellState {
    pub fn new() -> Self {
        Self {
            jobs: JobTable::new(),
            completions: HashMap::new(),
            history: vec![],
            history_append: 0,
            variables: HashMap::new(),
        }
    }
}

pub enum HistoryCmd {
    Show(Option<usize>),
    Read(String),
    Write(String),
    Append(String),
}

pub enum DeclareCmd {
    Print(String),
    Assign(String, String),
}

pub enum BuiltinCommand {
    Exit,
    Echo(Vec<String>),
    Type(String),
    Pwd,
    Cd(Option<String>),
    Complete(Vec<String>),
    History(HistoryCmd),
    Declare(DeclareCmd),
    Jobs,
    Unknown(String, Vec<String>),
}

pub const BUILTINS: &[&str] = &[
    "echo", "exit", "cd", "pwd", "type", "complete", "jobs", "history", "declare",
];

impl BuiltinCommand {
    pub fn parse(parts: &ParsedCommand) -> Self {
        match parts.args.as_slice() {
            [] => BuiltinCommand::Unknown(String::new(), vec![]),
            [cmd, rest @ ..] => Self::from_name(cmd.as_str(), rest),
        }
    }

    fn from_name(cmd: &str, args: &[String]) -> Self {
        match cmd {
            "exit" => BuiltinCommand::Exit,
            "echo" => BuiltinCommand::Echo(args.to_vec()),
            "type" => BuiltinCommand::Type(args.join(" ")),
            "pwd" => BuiltinCommand::Pwd,
            "cd" => BuiltinCommand::Cd(args.first().cloned()),
            "complete" => BuiltinCommand::Complete(args.to_vec()),
            "history" => {
                let cmd = match args {
                    [flag, path] if flag == "-r" => HistoryCmd::Read(path.clone()),
                    [flag, path] if flag == "-w" => HistoryCmd::Write(path.clone()),
                    [flag, path] if flag == "-a" => HistoryCmd::Append(path.clone()),
                    [n] => HistoryCmd::Show(n.parse::<usize>().ok()),
                    _ => HistoryCmd::Show(None),
                };
                BuiltinCommand::History(cmd)
            }
            "declare" => {
                let cmd = match args {
                    [flag, name] if flag == "-p" => DeclareCmd::Print(name.clone()),
                    [assignment] if assignment.contains('=') => {
                        //TODO: change the unwrap for a better way of handling this:
                        let (name, value) = assignment.split_once('=').unwrap();
                        DeclareCmd::Assign(name.to_string(), value.to_string())
                    }
                    _ => DeclareCmd::Print(String::new()),
                };
                BuiltinCommand::Declare(cmd)
            }

            "jobs" => BuiltinCommand::Jobs,
            _ => BuiltinCommand::Unknown(cmd.to_string(), args.to_vec()),
        }
    }

    pub fn execute(
        self,
        stdout_redirect: Option<Redirect>,
        stderr_redirect: Option<Redirect>,
        state: &mut ShellState,
        background: bool,
    ) {
        match self {
            BuiltinCommand::Exit => {
                if let Ok(histfile) = std::env::var("HISTFILE") {
                    let _ = history_write_file(&histfile, &state.history, false);
                }
                std::process::exit(0)
            }
            BuiltinCommand::Cd(path) => {
                if let Err(err) = cd(path) {
                    Self::write_stderr(&err, &stderr_redirect);
                }
            }
            BuiltinCommand::Echo(args) => {
                let output = echo(&args);
                Self::write_output(output, &stdout_redirect, &stderr_redirect);
            }
            BuiltinCommand::Type(arg) => {
                let output = type_cmd(arg);
                Self::write_output(output, &stdout_redirect, &stderr_redirect);
            }
            BuiltinCommand::Pwd => {
                let output = pwd();
                Self::write_output(output, &stdout_redirect, &stderr_redirect);
            }
            BuiltinCommand::Complete(args) => {
                let output = complete(&args, &mut state.completions);
                Self::write_output(output, &stdout_redirect, &stderr_redirect);
            }
            BuiltinCommand::History(cmd) => match cmd {
                HistoryCmd::Show(n) => {
                    let output = history(&state.history, n);
                    Self::write_output(output, &stdout_redirect, &stderr_redirect);
                }
                HistoryCmd::Read(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        state.history.extend(
                            content
                                .lines()
                                .filter(|l| !l.is_empty())
                                .map(|l| l.to_string()),
                        );
                    }
                    Err(e) => {
                        Self::write_stderr(&format!("history: {path}: {e}\n"), &stderr_redirect);
                    }
                },
                HistoryCmd::Write(path) => {
                    if let Err(e) = history_write_file(&path, &state.history, false) {
                        Self::write_stderr(&format!("history: {path}: {e}\n"), &stderr_redirect);
                    } else {
                        state.history_append = state.history.len();
                    }
                }
                HistoryCmd::Append(path) => {
                    if let Err(e) =
                        history_write_file(&path, &state.history[state.history_append..], true)
                    {
                        Self::write_stderr(&format!("history: {path}: {e}\n"), &stderr_redirect);
                    } else {
                        state.history_append = state.history.len();
                    }
                }
            },
            BuiltinCommand::Declare(cmd) => {
                let output = match cmd {
                    DeclareCmd::Print(name) => declare_print(&name, &state.variables),
                    DeclareCmd::Assign(name, value) => {
                        declare_assign(name, value, &mut state.variables)
                    }
                };
                Self::write_output(output, &stdout_redirect, &stderr_redirect);
            }
            BuiltinCommand::Jobs => {
                let output = jobs_list(&mut state.jobs);
                Self::write_output(output, &stdout_redirect, &stderr_redirect);
            }
            BuiltinCommand::Unknown(cmd, args) => {
                let result = run_external_cmd(cmd, &args, background, &mut state.jobs);
                match result {
                    ExecResult::Completed(output) => {
                        Self::write_output(output, &stdout_redirect, &stderr_redirect);
                    }
                    ExecResult::Background => {
                        // Job ya registrado, [id] pid ya impreso por JobTable::add
                    }
                }
            }
        }
    }

    fn write_output(
        output: CmdOutput,
        stdout_redirect: &Option<Redirect>,
        stderr_redirect: &Option<Redirect>,
    ) {
        // Handle stdout redirect
        if let Some(redirect) = stdout_redirect {
            match Self::open_redirect(redirect) {
                Ok(mut file) => {
                    if let Err(e) = file.write_all(output.stdout.as_bytes()) {
                        Self::write_stderr(&format!("redirect {e}"), stderr_redirect);
                    }
                }
                Err(e) => Self::write_stderr(&format!("redirect {e}"), stderr_redirect),
            }
        } else {
            print!("{}", output.stdout);
        }

        // Handle stderr redirect: always create/truncate the file if redirect is set,
        // even if stderr is empty (real shell behavior: redirection sets up the file)
        if let Some(redirect) = stderr_redirect {
            match Self::open_redirect(redirect) {
                Ok(mut file) => {
                    let _ = file.write_all(output.stderr.as_bytes());
                }
                Err(e) => eprint!("redirect {e}"),
            }
        } else if !output.stderr.is_empty() {
            eprint!("{}", output.stderr);
        }
    }

    fn write_stderr(msg: &str, redirect: &Option<Redirect>) {
        match redirect {
            Some(r) => {
                if let Ok(mut file) = Self::open_redirect(r) {
                    let _ = file.write_all(msg.as_bytes());
                } else {
                    eprint!("{msg}")
                }
            }
            None => eprint!("{msg}"),
        }
    }

    fn open_redirect(redirect: &Redirect) -> std::io::Result<File> {
        match redirect.mode {
            RedirectMode::Overwrite => File::create(&redirect.path),
            RedirectMode::Append => OpenOptions::new()
                .append(true)
                .create(true)
                .open(&redirect.path),
        }
    }

    pub fn is_builtin(cmd: &str) -> bool {
        !matches!(Self::from_name(cmd, &[]), BuiltinCommand::Unknown(_, _))
    }
}

pub fn spawn_builtin(
    cmd: BuiltinCommand,
    state: Arc<Mutex<ShellState>>,
) -> std::io::Result<(std::io::PipeReader, std::thread::JoinHandle<()>)> {
    let (reader, mut writer) = std::io::pipe()?;
    let handle = std::thread::spawn(move || {
        let output = match cmd {
            BuiltinCommand::Echo(args) => echo(&args),
            BuiltinCommand::Pwd => pwd(),
            BuiltinCommand::Type(args) => type_cmd(args),
            BuiltinCommand::Jobs => {
                let Ok(mut state) = state.lock() else {
                    return;
                };
                jobs_list(&mut state.jobs)
            }
            _ => CmdOutput::default(),
        };

        let _ = writer.write_all(output.stdout.as_bytes());
    });

    Ok((reader, handle))
}

pub fn execute_pipeline(pipeline: &mut crate::args::Pipeline, state: Arc<Mutex<ShellState>>) {
    let n = pipeline.commands.len();

    if n == 1 {
        let parsed = pipeline.commands.remove(0);
        let cmd = BuiltinCommand::parse(&parsed);
        if let Ok(mut s) = state.lock() {
            cmd.execute(
                parsed.stdout_redirect,
                parsed.stderr_redirect,
                &mut s,
                parsed.background,
            );
        } else {
            eprintln!("shell: internal error, exiting");
            std::process::exit(1);
        }
        return;
    }

    let mut children = vec![];
    let mut prev_stdout: Option<Stdio> = None;

    for (i, parsed) in pipeline.commands.iter().enumerate() {
        let is_last = i == n - 1;
        let cmd_name = parsed.args.first().map(|s| s.as_str()).unwrap_or("");

        if BuiltinCommand::is_builtin(cmd_name) {
            let cmd = BuiltinCommand::parse(parsed);
            if is_last {
                if let Ok(mut s) = state.lock() {
                    cmd.execute(
                        parsed.stdout_redirect.clone(),
                        parsed.stderr_redirect.clone(),
                        &mut s,
                        false,
                    );
                }

                break;
            }
            match spawn_builtin(cmd, Arc::clone(&state)) {
                Ok((reader, handle)) => {
                    prev_stdout = Some(reader.into());
                    children.push(PipelineChild::Thread(handle));
                    continue;
                }
                Err(e) => {
                    eprintln!("{e}");
                    return;
                }
            }
        }

        let Some(cmd_path) = get_path_cmd(cmd_name) else {
            eprintln!("{cmd_name}: command not found");
            return;
        };

        let stdin_cfg = match prev_stdout.take() {
            Some(stdout) => stdout,
            None => Stdio::inherit(),
        };

        let stdout_cfg = if is_last {
            // Last one: Apply redirect if requested, if not, inherit
            if let Some(ref redirect) = parsed.stdout_redirect {
                let file = match redirect.mode {
                    RedirectMode::Overwrite => File::create(&redirect.path),
                    RedirectMode::Append => OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&redirect.path),
                };
                match file {
                    Ok(f) => Stdio::from(f),
                    Err(e) => {
                        eprintln!("redirect: {e}");
                        return;
                    }
                }
            } else {
                Stdio::inherit()
            }
        } else {
            Stdio::piped()
        };

        let args = &parsed.args[1..];

        let child = Command::new(&cmd_path)
            .arg0(cmd_name)
            .args(args)
            .stdin(stdin_cfg)
            .stdout(stdout_cfg)
            .spawn();

        match child {
            Ok(mut c) => {
                if !is_last {
                    prev_stdout = Some(c.stdout.take().unwrap().into());
                }
                children.push(PipelineChild::Process(c));
            }
            Err(e) => {
                eprintln!("{cmd_name}: {e}");
                return;
            }
        }
    }

    if let Some(last) = children.pop() {
        match last {
            PipelineChild::Process(mut c) => {
                let _ = c.wait();
            }
            PipelineChild::Thread(h) => {
                let _ = h.join();
            }
        }
    }

    for child in children.drain(..) {
        match child {
            PipelineChild::Process(mut c) => {
                let _ = c.wait();
            }
            PipelineChild::Thread(h) => {
                let _ = h.join();
            }
        }
    }
}

pub fn echo(args: &[String]) -> CmdOutput {
    CmdOutput {
        stdout: format!("{}\n", args.join(" ")),
        ..Default::default()
    }
}

pub fn pwd() -> CmdOutput {
    CmdOutput {
        stdout: current_dir()
            .map(|p| format!("{}\n", p.display()))
            .unwrap_or_default(),
        ..Default::default()
    }
}

pub fn type_cmd(arg: String) -> CmdOutput {
    if BuiltinCommand::is_builtin(&arg) {
        CmdOutput {
            stdout: format!("{arg} is a shell builtin\n"),
            ..Default::default()
        }
    } else if let Some(path) = get_path_cmd(&arg) {
        CmdOutput {
            stdout: format!("{arg} is {}\n", path.display()),
            ..Default::default()
        }
    } else {
        CmdOutput {
            stdout: format!("{arg}: not found\n"),
            ..Default::default()
        }
    }
}

pub fn cd(path: Option<String>) -> Result<(), String> {
    let target = match path.as_deref() {
        None | Some("~") => {
            let Some(home) = env::home_dir() else {
                return Err("cd: Couldn't determinate the home directory\n".to_string());
            };
            home
        }
        Some(p) => PathBuf::from(p),
    };

    if env::set_current_dir(&target).is_err() {
        Err(format!(
            "cd: {}: No such file or directory\n",
            target.display()
        ))
    } else {
        Ok(())
    }
}

pub fn complete(args: &[String], completions: &mut HashMap<String, String>) -> CmdOutput {
    match args {
        [flag, completer, command] if flag == "-C" => {
            completions.insert(command.clone(), completer.clone());
            CmdOutput::default()
        }
        [flag] if flag == "-C" => CmdOutput {
            stderr: "complete: -C: no command specified\n".to_string(),
            ..Default::default()
        },
        [flag, command] if flag == "-p" => match completions.get(command.as_str()) {
            Some(completer) => CmdOutput {
                stdout: format!("complete -C '{completer}' {command}\n"),
                ..Default::default()
            },
            None => CmdOutput {
                stderr: format!("complete: {command}: no completion specification\n"),
                ..Default::default()
            },
        },
        [flag, command] if flag == "-r" => {
            completions.remove(command.as_str());
            CmdOutput::default()
        }

        [flag] if flag == "-r" => CmdOutput {
            stderr: "complete: -r: no command specified\n".to_string(),
            ..Default::default()
        },
        [flag] if flag == "-p" => CmdOutput {
            stderr: "complete: -p: no command specified\n".to_string(),
            ..Default::default()
        },
        [] => CmdOutput {
            stderr: "complete: usage: complete [-p] [command]\n".to_string(),
            ..Default::default()
        },

        _ => CmdOutput {
            stderr: format!("complete: {}: option not implemented\n", args[0]),
            ..Default::default()
        },
    }
}

fn history(commands: &[String], limit: Option<usize>) -> CmdOutput {
    let start = match limit {
        Some(n) if n < commands.len() => commands.len() - n,
        _ => 0,
    };

    let mut output = String::new();
    for (i, c) in commands.iter().enumerate().skip(start) {
        output.push_str(&format!("{:5}  {c}\n", i + 1));
    }

    CmdOutput {
        stdout: output,
        ..Default::default()
    }
}

fn history_write_file(path: &str, entries: &[String], append: bool) -> std::io::Result<()> {
    let content: String = entries.iter().map(|l| format!("{l}\n")).collect();
    if append {
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .and_then(|mut f| f.write_all(content.as_bytes()))
    } else {
        std::fs::write(path, content)
    }
}

fn declare_print(name: &str, variables: &HashMap<String, String>) -> CmdOutput {
    match variables.get(name) {
        Some(value) => CmdOutput {
            stdout: format!("declare -- {name}=\"{value}\"\n"),
            ..Default::default()
        },
        None => CmdOutput {
            stderr: format!("declare: {name}: not found\n"),
            ..Default::default()
        },
    }
}

fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }

    chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn declare_assign(
    name: String,
    value: String,
    variables: &mut HashMap<String, String>,
) -> CmdOutput {
    if !is_valid_identifier(&name) {
        return CmdOutput {
            stderr: format!("declare: `{name}={value}': not a valid identifier\n"),
            ..Default::default()
        };
    }
    variables.insert(name, value);
    CmdOutput::default()
}

fn jobs_list(jobs: &mut JobTable) -> CmdOutput {
    jobs.update_statuses();

    let mut stdout = String::new();
    let len = jobs.jobs.len();

    for (i, job) in jobs.jobs.iter().enumerate() {
        let symbol = if i + 1 == len {
            '+'
        } else if i + 2 == len {
            '-'
        } else {
            ' '
        };

        stdout.push_str(&format!("{}\n", job.format_line(symbol)));
    }

    jobs.jobs
        .retain(|job| !matches!(job.status, JobStatus::Done(_)));

    CmdOutput {
        stdout,
        ..Default::default()
    }
}

fn run_external_cmd(
    cmd: String,
    args: &[String],
    background: bool,
    jobs: &mut JobTable,
) -> ExecResult {
    let Some(cmd_path) = get_path_cmd(&cmd) else {
        return ExecResult::Completed(CmdOutput {
            stdout: String::new(),
            stderr: format!("{cmd}: command not found\n"),
        });
    };

    if background {
        let full_cmd = if args.is_empty() {
            cmd.clone()
        } else {
            format!("{} {}", cmd, args.join(" "))
        };
        match Command::new(&cmd_path).arg0(&cmd).args(args).spawn() {
            Ok(child) => {
                jobs.add(full_cmd, child);
                ExecResult::Background
            }
            Err(e) => ExecResult::Completed(CmdOutput {
                stdout: String::new(),
                stderr: format!("{cmd}: {e}\n"),
            }),
        }
    } else {
        match Command::new(&cmd_path).arg0(&cmd).args(args).output() {
            Ok(output) => ExecResult::Completed(CmdOutput {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
            Err(e) => ExecResult::Completed(CmdOutput {
                stdout: String::new(),
                stderr: format!("{cmd}: {e}\n"),
            }),
        }
    }
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

pub fn get_path_cmd(arg: &str) -> Option<PathBuf> {
    if let Ok(path) = var("PATH") {
        let dirs = path.split(":");
        for dir in dirs {
            let mut path_cmd = PathBuf::from(dir);
            path_cmd.push(arg);

            if path_cmd.exists() && is_executable(&path_cmd) {
                return Some(path_cmd);
            }
        }
    }
    None
}
