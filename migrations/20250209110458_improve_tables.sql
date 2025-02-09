-- Improve member lookup by using keycodes as the primary key, which will use
-- an index scan instead of a full table scan.

create table members_new
(
    keycode text not null primary key,
    id text not null,
    firstname text not null,
    lastname text not null,
    nickname text not null
);

insert into members_new (keycode, id, firstname, lastname, nickname)
select json_each.value, members.id, members.firstname, members.lastname, members.nickname
from members, json_each(members.keycodes);

drop table members;

alter table members_new rename to members;

-- Drop the redundant `articles.barcode` column since it is equivalent
-- to `articles.id`.

alter table articles drop column barcode;