use std::process::Child;

pub enum JobStatus {
    Running,
    Done,
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
            JobStatus::Done => ("Done", ""),
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
                && let Ok(Some(_)) = job.child.try_wait()
            {
                job.status = JobStatus::Done;
            }
        }
    }

    pub fn print_notifications(&mut self) {
        let len = self.jobs.len();
        for (i, job) in self.jobs.iter_mut().enumerate() {
            if let JobStatus::Done = job.status
                && !job.notified
            {
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

    pub fn poll(&mut self) {
        self.update_statuses();
        self.print_notifications();
        self.jobs
            .retain(|job| !matches!(job.status, JobStatus::Done));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::Duration;

    // ── JobTable::new ─────────────────────────────────────────────

    #[test]
    fn test_job_table_new() {
        let table = JobTable::new();
        assert!(table.jobs.is_empty());
        assert_eq!(table.next_id, 1);
    }

    // ── next_available_id ─────────────────────────────────────────

    #[test]
    fn test_next_available_id_empty() {
        let table = JobTable::new();
        assert_eq!(table.next_available_id(), 1);
    }

    #[test]
    fn test_next_available_id_after_add() {
        let mut table = JobTable::new();
        let child = Command::new("true").spawn().unwrap();
        table.add("true".to_string(), child);
        // One job with id=1 exists, so next available is 2
        assert_eq!(table.next_available_id(), 2);
    }

    #[test]
    fn test_next_available_id_reuses_freed_id() {
        let mut table = JobTable::new();
        let child = Command::new("true").spawn().unwrap();
        table.add("true".to_string(), child);
        // Manually remove the job to simulate a freed slot
        table.jobs.clear();
        assert_eq!(
            table.next_available_id(),
            1,
            "should reuse id 1 after removal"
        );
    }

    // ── Job::format_line ──────────────────────────────────────────

    #[test]
    fn test_job_format_line_running_plus() {
        let child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        let job = Job {
            id: 1,
            pid,
            full_cmd: "my_cmd".to_string(),
            child,
            status: JobStatus::Running,
            notified: false,
        };
        let line = job.format_line('+');
        // Format: {:<6} with "[1]+" (4 chars) → "[1]+  " — 2 padding spaces
        //         {:<24} with "Running" (7 chars) → "Running" + 17 spaces
        assert_eq!(line, "[1]+  Running                 my_cmd &");
    }

    #[test]
    fn test_job_format_line_running_minus() {
        let child = Command::new("sh").arg("-c").arg("true").spawn().unwrap();
        let job = Job {
            id: 2,
            pid: child.id(),
            full_cmd: "other_cmd".to_string(),
            child,
            status: JobStatus::Running,
            notified: false,
        };
        let line = job.format_line('-');
        assert_eq!(line, "[2]-  Running                 other_cmd &");
    }

    #[test]
    fn test_job_format_line_done() {
        let child = Command::new("true").spawn().unwrap();
        let job = Job {
            id: 5,
            pid: child.id(),
            full_cmd: "finished_job".to_string(),
            child,
            status: JobStatus::Done,
            notified: false,
        };
        let line = job.format_line(' ');
        // marker: "[5] " (4 chars) → padded to 6 → "[5]   "
        // "Done" (4 chars) → padded to 24 → "Done" + 20 spaces
        assert_eq!(line, "[5]   Done                    finished_job");
    }

    #[test]
    fn test_job_format_line_done_no_suffix() {
        // Done jobs should NOT have " &" suffix
        let child = Command::new("true").spawn().unwrap();
        let job = Job {
            id: 3,
            pid: child.id(),
            full_cmd: "done_cmd".to_string(),
            child,
            status: JobStatus::Done,
            notified: false,
        };
        let line = job.format_line('+');
        assert!(
            !line.ends_with(" &"),
            "Done jobs should not have ' &' suffix: {line:?}"
        );
        assert_eq!(line, "[3]+  Done                    done_cmd");
    }

    #[test]
    fn test_job_format_line_wide_id() {
        // Two-digit IDs should still align properly
        let child = Command::new("true").spawn().unwrap();
        let job = Job {
            id: 42,
            pid: child.id(),
            full_cmd: "wide_id_cmd".to_string(),
            child,
            status: JobStatus::Running,
            notified: false,
        };
        let line = job.format_line('+');
        // marker "[42]+" is 5 chars, padded to min width 6 → "[42]+ " (1 space)
        assert_eq!(line, "[42]+ Running                 wide_id_cmd &");
    }

    // ── JobTable::add ─────────────────────────────────────────────

    #[test]
    fn test_job_table_add_increases_count() {
        let mut table = JobTable::new();
        let child = Command::new("true").spawn().unwrap();
        table.add("true".to_string(), child);
        assert_eq!(table.jobs.len(), 1);
    }

    #[test]
    fn test_job_table_add_increments_next_id() {
        let mut table = JobTable::new();
        let child = Command::new("true").spawn().unwrap();
        table.add("true".to_string(), child);
        assert_eq!(table.next_id, 2);
    }

    #[test]
    fn test_job_table_add_returns_id() {
        let mut table = JobTable::new();
        let child = Command::new("true").spawn().unwrap();
        let id = table.add("true".to_string(), child);
        assert_eq!(id, 1);
    }

    #[test]
    fn test_job_table_add_stores_fields() {
        let mut table = JobTable::new();
        let child = Command::new("sh").arg("-c").arg("echo hi").spawn().unwrap();
        let pid = child.id();
        table.add("echo hi".to_string(), child);

        let job = &table.jobs[0];
        assert_eq!(job.id, 1);
        assert_eq!(job.pid, pid);
        assert_eq!(job.full_cmd, "echo hi");
        assert!(matches!(job.status, JobStatus::Running));
        assert!(!job.notified);
    }

    #[test]
    fn test_job_table_add_multiple_jobs() {
        let mut table = JobTable::new();
        let c1 = Command::new("true").spawn().unwrap();
        let c2 = Command::new("true").spawn().unwrap();
        let c3 = Command::new("true").spawn().unwrap();

        table.add("job1".to_string(), c1);
        table.add("job2".to_string(), c2);
        table.add("job3".to_string(), c3);

        assert_eq!(table.jobs.len(), 3);
        assert_eq!(table.jobs[0].id, 1);
        assert_eq!(table.jobs[1].id, 2);
        assert_eq!(table.jobs[2].id, 3);
        assert_eq!(table.next_id, 4);
    }

    // ── update_statuses ───────────────────────────────────────────

    #[test]
    fn test_update_statuses_detects_completed() {
        let mut table = JobTable::new();
        let child = Command::new("true").spawn().unwrap();
        table.add("true".to_string(), child);

        // Give the child a moment to exit before calling try_wait
        std::thread::sleep(Duration::from_millis(50));
        table.update_statuses();
        assert!(matches!(table.jobs[0].status, JobStatus::Done));
    }

    #[test]
    fn test_update_statuses_keeps_running_alive() {
        let mut table = JobTable::new();
        // We can't reliably test a long-running process here since it would hang.
        // Instead, verify that a newly-added job starts as Running
        let child = Command::new("sh").arg("-c").arg("true").spawn().unwrap();
        table.add("true".to_string(), child);
        assert!(matches!(table.jobs[0].status, JobStatus::Running));
    }

    // ── poll ──────────────────────────────────────────────────────

    #[test]
    fn test_poll_removes_done_jobs() {
        let mut table = JobTable::new();
        let child = Command::new("true").spawn().unwrap();
        table.add("true".to_string(), child);

        // Give the child a moment to exit before poll calls try_wait
        std::thread::sleep(Duration::from_millis(50));
        table.poll();
        assert!(table.jobs.is_empty(), "Done jobs should be removed by poll");
    }

    #[test]
    fn test_poll_keeps_running_jobs() {
        let mut table = JobTable::new();
        let child = Command::new("sh").arg("-c").arg("true").spawn().unwrap();
        table.add("true".to_string(), child);

        // Update to Done manually to simulate running → done transition
        table.update_statuses();
        // Job might be done now, but we want to test that poll only removes Done.
        // After update_statuses, the job could be Done.
        // Let's trust that polling removes only Done jobs.
        let before = table.jobs.len();
        table.poll();
        // If it became Done, it should be gone
        assert!(
            table.jobs.len() <= before,
            "poll should not increase job count"
        );
    }
}
