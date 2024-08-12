# runbench

cli tool useful when trying to run a function on many *problems* (files) where the time it takes the function to finish is quite unpredictable with respect to the input, and may take *too much* time or not terminate at all

produces structured output in a sqlite db, nice for querying

## setup


### create an sqlx database

(i suggest using sqlx cli tool - [link](<https://lib.rs/crates/sqlx-cli>))

```sh
sqlx migrate run # creates db, creates tables
```

or just create a new db & execute the commands in the [migration file](./migrations/20240723094928_create.sql)

## usage

`runbench` hooks you up with a placeholder string `@bench_file` for you to use in the command you want to test the "performance" of, and runs this command repeatedly with each file in the directory you specify

lets say we want to test the performance of the following *callable* (ensure it is executable, eg `chmod u+x` it)

```sh
./demo/your_program.sh some_file  # the script ingnores the input file, waits for 5s and exits
```

you can do so by running

```sh
cargo run -- run --run-name "your_run_name" --dir-path ./demo/dataset --command "./demo/your_program @bench_file"
```

## inspecting the results

you can do so by `select`ing on the `attempts` table

```sql
select * from attempts;
```

| id | run_id | input_file | timeout_seconds | success | time_used_seconds |
|-|-|-|-|-|-|
|1|1|./dataset/example0|1|0|1|
|2|1|./dataset/example1|1|0|1|
|3|1|./dataset/example0|2|0|2|
|4|1|./dataset/example1|2|0|2|
|5|1|./dataset/example0|4|0|4|
|6|1|./dataset/example1|4|0|4|
|7|1|./dataset/example0|8|1|5|
|8|1|./dataset/example1|8|1|5|

we can see that the time limits (`timeout_seconds`) of 1s, 2s, 4s were not enough (`success = 0`), but 8s were enough, as expected given the content of the [demo script](./demo/your_program.sh).

we can also get just the times of the datasets we have successfully found the time requirements for

```sql
select input_file, time_used_seconds from attempts where run_id = 1 and success = true;
```

|input_file|time_used_seconds|
|-|-|
|./dataset/example0|5|
|./dataset/example1|5|

another usecase is to run `runbench` with different scripts/solutions over the same input files. you can then compare the "performance" of the solutions via eg

```sql
select * from (select run_id, count(run_id) from attempts where success = true group by run_id) as data join runs on data.run_id = runs.id;
```

to see how much of the inputs each of the solutions managed to solve (including the "metadata" - name of the run by joining with `runs` table)
