create table if not exists darkreader_blacklist (
    id integer primary key autoincrement,
    host text not null
);

create unique index if not exists darkreader_blacklist_index on darkreader_blacklist(host);

delete from darkreader_blacklist;
insert into darkreader_blacklist (host) values ('github.com');
insert into darkreader_blacklist (host) values ('gitee.com');
insert into darkreader_blacklist (host) values ('chat.deepseek.com');
