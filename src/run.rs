use crate::run_config::RunConfig;
use sqlx::{Pool, Sqlite};
use std::cmp::min;
use std::collections::VecDeque;
use std::path::PathBuf;

pub struct PendingBenchmark {
    /// Input data; This is either an absolute path to a file, or a string provided by the
    /// input generator.
    input: String,
    /// If more than one repetition is requested, this variable tracks the number of remaining
    /// successful attempts.
    remaining_repetitions: usize,
    /// What timeout should be used for the next run of this benchmark. This value cannot exceed
    /// `absolute_timeout` if it is specified.
    current_timeout: usize,
    /// If this benchmark is running, stores the reference to the running command.
    running_job: Option<tokio::process::Child>,
}

impl PendingBenchmark {
    /// Check if the current running job is completed. If yes, update internal counters to
    /// reflect the job result and return `true`. Otherwise, return `false`.
    pub fn check_job_completion(&mut self, config: &RunConfig) -> bool {
        let job = self.running_job.as_mut().expect("No running job found.");
        match job.try_wait().unwrap() {
            Some(status) => {
                self.running_job = None;
                if status.success() {
                    self.remaining_repetitions -= 1;
                    println!(
                        "Benchmark `{}` success. Remaining repetitions: {}.",
                        self.input, self.remaining_repetitions
                    );
                } else if status.code() == Some(124) {
                    // Command timeout.
                    if let Some(absolute_timeout) = config.absolute_timeout {
                        if self.current_timeout == absolute_timeout {
                            println!(
                                "Benchmark `{}` timeout. Absolute limit ({}s) reached, won't retry.",
                                self.input, absolute_timeout
                            );
                            self.remaining_repetitions = 0; // Skip all remaining repetitions.
                            return true; // Early return to ensure we don't print the default message.
                        } else {
                            self.current_timeout = min(absolute_timeout, self.current_timeout * 2);
                        }
                    } else {
                        self.current_timeout *= 2;
                    }
                    println!(
                        "Benchmark `{}` timeout. New timeout: {}s.",
                        self.input, self.current_timeout
                    );
                } else {
                    self.remaining_repetitions -= 1;
                    println!(
                        "Benchmark `{}` failed (exit code {:?}). Remaining repetitions: {}",
                        self.input,
                        status.code(),
                        self.current_timeout
                    );
                }
                true
            }
            None => {
                // Still running.
                false
            }
        }
    }

    /// Start a job corresponding to this benchmark. Panics if a job is already running.
    pub fn start_job(&mut self, config: &RunConfig) {
        assert!(self.running_job.is_none());

        // TODO: This is not dealing with command output in any way.

        let bench_command = config.command_interpolation(self.input.as_str());

        let mut args = Vec::new();
        args.push("timeout".to_string());
        args.push(format!("{}s", self.current_timeout));
        args.extend(Self::build_time_command());
        args.extend(bench_command.clone());

        let mut command = tokio::process::Command::new(args[0].as_str());
        command.args(&args[1..]);
        command.kill_on_drop(true);

        if let Some(mem_limit) = config.memory_limit {
            Self::set_memory_limit(&mut command, mem_limit);
            println!(
                "Starting (timeout {}s; memory limit {}MiB): {}",
                self.current_timeout,
                mem_limit,
                bench_command.join(" ")
            );
        } else {
            println!(
                "Starting (timeout {}s): {}",
                self.current_timeout,
                bench_command.join(" ")
            );
        }

        self.running_job = Some(command.spawn().unwrap());
    }

    /// Check if this benchmark is fully completed and should not be executed again.
    pub fn is_fully_completed(&self) -> bool {
        self.remaining_repetitions == 0
    }

    #[cfg(target_os = "linux")]
    fn set_memory_limit(command: &mut tokio::process::Command, limit: u64) {
        unsafe {
            command.pre_exec(move || {
                let soft_limit = limit * 1024 * 1024;
                rlimit::setrlimit(rlimit::Resource::AS, soft_limit, soft_limit * 2).unwrap();
                Ok(())
            });
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn set_memory_limit(_command: &mut tokio::process::Command, _limit: u64) {
        println!("Memory limit is ignored. Not supported on this platform.");
    }

    #[cfg(target_os = "linux")]
    fn build_time_command() -> Vec<String> {
        vec!["time".to_string(), "-v".to_string()]
    }

    #[cfg(target_os = "macos")]
    fn build_time_command() -> Vec<String> {
        // Uses gnu time.
        // TODO: Print a user-friendly error if gtime is not installed (brew install gnu-time).
        vec!["gtime".to_string(), "-v".to_string()]
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn build_time_command() -> Vec<String> {
        // Use "default" time command with standard posix output format.
        vec!["time".to_string(), "-p".to_string()]
    }
}

pub struct Run<'a> {
    config: RunConfig,
    db_pool: &'a Pool<Sqlite>,
    pending_benchmarks: VecDeque<PendingBenchmark>,
    running_benchmarks: Vec<PendingBenchmark>,
}

impl<'a> Run<'a> {
    /// Prepare input data for a particular run.
    async fn generate_inputs(run_config: &RunConfig) -> Vec<String> {
        if run_config.input_data.is_empty() {
            panic!("No input data provided.")
        }

        // First, check if input data is a directory, and if so, use files in that directory.

        if run_config.input_data.len() == 1 {
            let candidate_path_str = run_config.input_data[0].clone();
            let candidate_path = PathBuf::from(candidate_path_str.clone());
            if candidate_path.is_dir() {
                println!("Provided input data seems to be a directory. Loading files...");
                let mut input_paths = Vec::new();
                for entry in std::fs::read_dir(candidate_path).unwrap() {
                    let entry = entry.unwrap();
                    let abs_path = std::path::absolute(entry.path()).unwrap();
                    input_paths.push(abs_path.display().to_string());
                }
                input_paths.sort();
                println!(
                    "Loaded {} input files from `{}`.",
                    input_paths.len(),
                    candidate_path_str
                );
                return input_paths;
            }
        }

        // If input data is not a directory, it means we want to run it like a command...

        println!("Provided input seems to be a command. Executing... ");
        println!("{}", run_config.input_data.join(" "));
        let mut input_cmd = tokio::process::Command::new(run_config.input_data[0].clone());
        input_cmd.args(&run_config.input_data[1..]);
        input_cmd.kill_on_drop(true);

        let input_data = input_cmd.output().await.unwrap();
        let input_data = String::from_utf8(input_data.stdout).unwrap();

        let input_data: Vec<String> = input_data.lines().map(|it| it.to_string()).collect();

        println!("Command generated {} input instances.", input_data.len());

        input_data
    }

    /// Create a new run, using a config and database pool.
    pub async fn new(config: RunConfig, db_pool: &'a Pool<Sqlite>) -> Self {
        assert!(config.parallelism > 0, "Parallelism must be at least one.");
        assert!(config.repetitions > 0, "Repetitions must be at least one.");
        assert!(
            config.test_command.iter().any(|it| it.contains("@input")),
            "Placeholder `@input` must appear in the test command."
        );

        let pending_benchmarks = Self::generate_inputs(&config)
            .await
            .into_iter()
            .map(|input| PendingBenchmark {
                input,
                remaining_repetitions: config.repetitions,
                current_timeout: 1,
                running_job: None,
            })
            .collect::<VecDeque<_>>();

        Run {
            config,
            db_pool,
            pending_benchmarks,
            running_benchmarks: vec![],
        }
    }

    /// Execute the run until completion.
    pub fn execute_all(&mut self) {
        // Number of milliseconds we will be waiting until we next check completed benchmarks.
        // This is dynamically updated based on how long it takes to finish computations.
        let mut delay_to_next_check: u64 = 16;

        // The goal of this loop is to try to run while the pending queue is not empty. Running
        // benchmarks are moved in and out of queue as necessary.
        while !self.pending_benchmarks.is_empty() || !self.running_benchmarks.is_empty() {
            // First, go through all the running benchmarks and process all that finished.
            let running_before = self.running_benchmarks.len();
            let mut still_running = Vec::new();
            while let Some(mut running) = self.running_benchmarks.pop() {
                if running.check_job_completion(&self.config) {
                    // Job completed. Either it is completely finished, or we need to put it
                    // back into the queue.
                    if !running.is_fully_completed() {
                        self.pending_benchmarks.push_back(running);
                    }
                } else {
                    still_running.push(running);
                }
            }
            let running_after = still_running.len();
            self.running_benchmarks = still_running;

            // If some job was completed, we will check again in 16ms, otherwise we will
            // gradually wait longer and longer, up to one full second.
            if running_after < running_before {
                delay_to_next_check = 16;
            } else {
                delay_to_next_check = min(1000, delay_to_next_check * 2);
            }

            while self.running_benchmarks.len() < self.config.parallelism {
                if let Some(mut pending) = self.pending_benchmarks.pop_front() {
                    // Found an extra job that we can start.
                    pending.start_job(&self.config);
                    self.running_benchmarks.push(pending);
                } else {
                    // No more jobs to start.
                    break;
                }
            }

            // Wait a while before checking the tasks again.
            std::thread::sleep(std::time::Duration::from_millis(delay_to_next_check));
        }
    }
}
