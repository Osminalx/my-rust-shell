use std::process::Child;

pub enum JobStatus {
    Running,
    Done(i32),
}

pub struct Job {
    pub id: usize,
    pub pid: u32,
    pub full_cmd: String,
    pub child: Child,
    pub status: JobStatus,
    pub notified: bool,
}

pub struct JobTable {
    pub jobs: Vec<Job>,
    pub next_id: usize,
}

impl Job {
    pub fn format_line(&self, symbol: char) -> String {
        let marker = format!("[{}]{}", self.id, symbol);

        let (status_str, suffix) = match self.status {
            JobStatus::Running => ("Running", " &"),
            JobStatus::Done(_) => ("Done", ""),
        };
        format!("{:<6}{:<24}{}{}", marker, status_str, self.full_cmd, suffix)
    }
}

impl JobTable {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_id: 1,
        }
    }

    pub fn next_available_id(&self) -> usize {
        let mut id = 1;
        loop {
            if !self.jobs.iter().any(|j| j.id == id) {
                return id;
            }
            id += 1;
        }
    }

    pub fn add(&mut self, full_cmd: String, child: Child) -> usize {
        let curr_id = self.next_available_id();
        let curr_job = Job {
            id: curr_id,
            pid: child.id(),
            full_cmd,
            child,
            status: JobStatus::Running,
            notified: false,
        };
        println!("[{}] {}", &curr_job.id, &curr_job.pid);
        self.jobs.push(curr_job);
        self.next_id += 1;
        curr_id
    }

    pub fn update_statuses(&mut self) {
        for job in &mut self.jobs {
            if let JobStatus::Running = job.status
                && let Ok(Some(status)) = job.child.try_wait()
            {
                job.status = JobStatus::Done(status.code().unwrap_or(0));
            }
        }
    }

    pub fn print_notifications(&mut self) {
        let len = self.jobs.len();
        for (i, job) in self.jobs.iter_mut().enumerate() {
            if let JobStatus::Done(_) = job.status {
                if !job.notified {
                    let symbol = if i + 1 == len {
                        '+'
                    } else if i + 2 == len {
                        '-'
                    } else {
                        ' '
                    };
                    println!("{}", job.format_line(symbol));
                    job.notified = true;
                }
            }
        }
    }

    pub fn poll(&mut self) {
        self.update_statuses();
        self.print_notifications();
        self.jobs
            .retain(|job| !matches!(job.status, JobStatus::Done(_)));
    }
}
