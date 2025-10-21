create table if not exists navigation_log (
    id integer primary key autoincrement,
    url text not null,
    title text not null,
    icon_id integer,
    star boolean not null default 0,
    times integer not null,
    last_time datetime not null
);

create unique index if not exists navigation_log_url_index on navigation_log(url);
create index if not exists navigation_log_title_index on navigation_log(title);

create table if not exists icon_cached (
    id integer primary key autoincrement,
    url text not null,
    data_url text,
    update_time datetime not null
);

create unique index if not exists icon_cached_url_index on icon_cached(url);
