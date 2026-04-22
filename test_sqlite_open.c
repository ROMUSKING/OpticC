#include <stdio.h>
typedef struct sqlite3 sqlite3;
extern int sqlite3_open(const char *filename, sqlite3 **pp_db);
extern const char *sqlite3_errstr(int);
int main() {
    sqlite3 *db = 0;
    int rc = sqlite3_open(":memory:", &db);
    printf("rc=%d err=%s db=%p\n", rc, sqlite3_errstr(rc), db);
    return 0;
}
