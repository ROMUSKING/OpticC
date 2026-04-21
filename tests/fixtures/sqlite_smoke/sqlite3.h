typedef struct sqlite3 sqlite3;
typedef int (*sqlite3_callback)(void *arg, int columns, char **values, char **names);

int sqlite3_open(const char *filename, sqlite3 **pp_db);
int sqlite3_exec(
    sqlite3 *db,
    const char *sql,
    sqlite3_callback callback,
    void *arg,
    char **errmsg
);
int sqlite3_close(sqlite3 *db);
const char *sqlite3_libversion(void);
