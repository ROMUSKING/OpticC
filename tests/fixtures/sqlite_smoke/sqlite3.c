#include "sqlite3.h"

struct sqlite3 {
    int open;
    int statements;
};

int sqlite3_open(const char *filename, sqlite3 **pp_db) {
    static struct sqlite3 db;
    (void)filename;
    if (!pp_db) {
        return 1;
    }
    db.open = 1;
    db.statements = 0;
    *pp_db = &db;
    return 0;
}

int sqlite3_exec(
    sqlite3 *db,
    const char *sql,
    sqlite3_callback callback,
    void *arg,
    char **errmsg
) {
    (void)errmsg;
    if (!db || !sql || !db->open) {
        return 1;
    }
    db->statements += 1;
    if (callback) {
        return callback(arg, 0, 0, 0);
    }
    return 0;
}

int sqlite3_close(sqlite3 *db) {
    if (!db || !db->open) {
        return 1;
    }
    db->open = 0;
    return 0;
}

const char *sqlite3_libversion(void) {
    return "optic-fixture";
}
