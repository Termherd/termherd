+++
id = "F-store-cache"
type = "feature"
area = ["sessions"]
status = "todo"
target = ["Should"]
+++

A SQLite digest cache with an FTS5 index, replacing the in-memory scan.

SQLite (WAL) digest cache + FTS5 index (lowest Should priority; an optimisation
over the in-memory scan/search)
