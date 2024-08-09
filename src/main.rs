use std::process::Output;

use anyhow::Result;
use clap::{Parser, Subcommand};
use sqlx::sqlite::SqlitePoolOptions;

const FILE_PLACEHOLDER: &str = "@bench_file";

#[derive(Subcommand)]
enum Command {
    Run {
        #[clap(short, long)]
        run_name: String,
        #[clap(short, long)]
        dir_path: String,
        #[clap(short, long)]
        command: String,
        #[clap(short, long)]
        final_cutoff_seconds: Option<usize>,
    },
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

async fn create_pool() -> Result<sqlx::Pool<sqlx::Sqlite>> {
    // load the url at comptime
    let url = dotenvy_macro::dotenv!("DATABASE_URL");

    Ok(SqlitePoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await?)
}

#[derive(Debug, sqlx::FromRow)]
#[allow(unused)] // todo remove once the fields are used (in a way the compiler recognizes anyway)
struct Run {
    id: i64,
    time_started: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let Command::Run {
        run_name,
        dir_path,
        command,
        final_cutoff_seconds,
    } = Cli::parse().command;

    let pool = create_pool().await?;

    // sometimes, the write is not persisted to the db if not behind a transaction for some reason
    // todo remove the tx (correct the ^issue) to improve performance
    let mut tx = pool.begin().await?;
    let run_id = sqlx::query_scalar!("insert into runs (name) values (?) returning id;", run_name)
        .fetch_one(tx.as_mut())
        .await?;
    tx.commit().await?;

    println!("RUNNING BENCH WITH run_id = {}", run_id);

    let files = std::fs::read_dir(dir_path)?
        .map(|entry| entry.unwrap().path().to_str().unwrap().to_owned())
        .collect::<Vec<_>>();

    bench_loop(
        run_id,
        &files.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        &command,
        final_cutoff_seconds,
        &pool,
    )
    .await;

    Ok(())
}

async fn bench_loop(
    run_id: i64,
    files: &[&str],
    command_with_placeholder: &str,
    final_cutoff_seconds: Option<usize>,
    pool: &sqlx::Pool<sqlx::Sqlite>,
) {
    let timeouts_seconds = (0..32) // 2^32 = 100+ years
        .map(|i| 2usize.pow(i))
        .take_while(|&x| final_cutoff_seconds.map_or(true, |cutoff| x < cutoff))
        .chain(final_cutoff_seconds);

    for timeout_seconds in timeouts_seconds {
        for file in files.iter() {
            let exists = sqlx::query_scalar!(
                "select exists (select 1 from attempts where run_id = ? and input_file = ? and success = 1);",
                run_id,
                file
            )
            .fetch_one(pool)
            .await
            .expect("should not fail");

            if exists == 1 {
                println!("skipping: {} {}; solved already", file, timeout_seconds);
                continue;
            }

            let now = std::time::Instant::now();
            let out = build_and_execute_cmd(timeout_seconds, file, command_with_placeholder);
            let time_used_seconds = now.elapsed().as_secs() as i64;

            let db_timeout_seconds = timeout_seconds as i64;
            let db_success = out.status.success();
            let stdout = out.stdout;
            let stderr = out.stderr;

            let mut tx = pool
                .begin()
                .await
                .expect("transaction should be possible to start");

            sqlx::query!("insert into attempts (run_id, input_file, timeout_seconds, success, time_used_seconds, stdout, stderr) values (?, ?, ?, ?, ?, ?, ?);",
                run_id, file, db_timeout_seconds, db_success, time_used_seconds, stdout, stderr)
                .execute(tx.as_mut())
                .await
                .expect("insert should not fail");

            tx.commit().await.expect("commit should not fail");
        }
    }
}

fn build_and_execute_cmd(
    timeout_seconds: usize,
    file: &str,
    command_with_placeholder: &str,
) -> Output {
    let interpolated_cmd = command_with_placeholder.replace(FILE_PLACEHOLDER, file);

    let time_restricted_interpolated_cmd = format!("timeout {timeout_seconds}s {interpolated_cmd}");

    println!("running: {}", time_restricted_interpolated_cmd);

    std::process::Command::new("sh")
        .arg("-c")
        .arg(time_restricted_interpolated_cmd)
        .output()
        .expect("failed to execute process")
}
