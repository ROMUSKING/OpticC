int sqlite3_open(const char *filename, void **ppDb) {
    (void)filename;
    (void)ppDb;
    return 0;
}
int sqlite3_close(void *db) {
    (void)db;
    return 0;
}
int sqlite3_exec(void *db, const char *sql, void *callback, void *arg, char **errmsg) {
    (void)db;
    (void)sql;
    (void)callback;
    (void)arg;
    (void)errmsg;
    return 0;
}
const char *sqlite3_libversion(void) {
    return "3.49.2";
}
const char *sqlite3_sourceid(void) {
    return "mock-2026-01-01";
}
