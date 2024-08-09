PRAGMA foreign_keys = ON;

create table if not exists runs (
    id integer not null primary key autoincrement,
    'name' text not null,
    time_started text not null default current_timestamp -- string is love string is life
);

create table if not exists attempts (
    id integer primary key autoincrement,
    run_id integer not null,

    input_file text not null default '',
    timeout_seconds integer not null,
    success boolean not null,
    time_used_seconds integer not null, -- value not defined if not succeeded

    stdout text not null,
    stderr text not null, -- to see those that failed for other reason than the timeout
    
    foreign key(run_id) references runs(id)
);
