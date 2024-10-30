use runbench::run::Run;
use runbench::run_config::RunConfig;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() {
    let config = RunConfig {
        name: "simple-test".to_string(),
        test_command: vec!["sleep".to_string(), "@input".to_string()],
        input_data: vec!["./test-inputs/gen-powers.sh".to_string(), "5".to_string()],
        raw_output: None,
        parallelism: 1,
        repetitions: 1,
        memory_limit: Some(1),
        absolute_timeout: None,
    };
    // load the url at comptime
    let url = dotenvy_macro::dotenv!("DATABASE_URL");

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .unwrap();

    let mut run = Run::new(config, &pool).await;
    run.execute_all();
}
